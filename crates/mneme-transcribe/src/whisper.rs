//! [`WhisperEngine`]: whisper.cpp through the `whisper-rs` crate, on the GPU
//! (Metal) where available.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use crate::text::clean_segment_text;
use crate::transcriber::{Language, TextSpan, TranscribeError, Transcriber, duration_ms};

/// How to load a model. `WhisperOptions::default()` is what the app uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperOptions {
    /// Run the model on the GPU through Metal. whisper.cpp falls back to the
    /// CPU by itself when no GPU can be used. Default `true`.
    pub use_gpu: bool,
    /// Number of CPU threads. `None` keeps whisper.cpp's default, which is
    /// the number of cores up to four. Default `None`.
    pub threads: Option<u32>,
}

impl Default for WhisperOptions {
    fn default() -> Self {
        WhisperOptions {
            use_gpu: true,
            threads: None,
        }
    }
}

/// A loaded Whisper model.
///
/// Loading reads the whole model file and sets up the GPU, so load once per
/// recording and reuse the engine for every chunk. Calls to
/// [`transcribe`](Transcriber::transcribe) from several threads run one after
/// another.
///
/// Drop the engine before the process exits. It must not live in a `static`
/// or be leaked, and an app must drop it before calling `std::process::exit`:
/// whisper.cpp's Metal backend checks at exit that every model was freed and
/// aborts the process if one was not.
///
/// whisper.cpp writes its own log lines (model details, the backend in use)
/// to stderr. The first load on a machine can take several seconds longer
/// than later ones while macOS compiles the Metal shaders; the result is
/// cached by the system.
pub struct WhisperEngine {
    model_path: PathBuf,
    multilingual: bool,
    threads: Option<u32>,
    /// Decoder working memory, allocated once and reused by every call. It
    /// keeps the loaded model alive. The lock is what serializes calls.
    state: Mutex<WhisperState>,
}

impl WhisperEngine {
    /// Loads the model file at `model_path` (a whisper.cpp `ggml-*.bin`
    /// file) with the default options.
    pub fn load(model_path: impl AsRef<Path>) -> Result<Self, TranscribeError> {
        Self::load_with(model_path, &WhisperOptions::default())
    }

    /// Loads the model file at `model_path` with `options`.
    pub fn load_with(
        model_path: impl AsRef<Path>,
        options: &WhisperOptions,
    ) -> Result<Self, TranscribeError> {
        let model_path = model_path.as_ref().to_path_buf();
        if !model_path.is_file() {
            return Err(TranscribeError::ModelNotFound { path: model_path });
        }

        let mut parameters = WhisperContextParameters::default();
        parameters.use_gpu(options.use_gpu);

        let load_error = |what: &str, error: whisper_rs::WhisperError| TranscribeError::ModelLoad {
            path: model_path.clone(),
            message: format!("{what} ({error}) The file may be damaged or not a Whisper model."),
        };
        let context = WhisperContext::new_with_params(&model_path, parameters)
            .map_err(|e| load_error("whisper.cpp could not read the file", e))?;
        let multilingual = context.is_multilingual();
        let state = context
            .create_state()
            .map_err(|e| load_error("whisper.cpp could not set up the model", e))?;

        Ok(WhisperEngine {
            model_path,
            multilingual,
            threads: options.threads,
            state: Mutex::new(state),
        })
    }

    /// The file this model was loaded from.
    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    /// False for English-only models (`*.en`).
    pub fn is_multilingual(&self) -> bool {
        self.multilingual
    }

    /// Checks that this model can transcribe `language`, without running it.
    ///
    /// [`transcribe`](Transcriber::transcribe) makes the same check on every
    /// call; this lets a caller refuse a bad setting before recording starts.
    pub fn check_language(&self, language: &Language) -> Result<(), TranscribeError> {
        resolve_language(self.multilingual, language).map(|_| ())
    }

    /// The whisper.cpp version this crate was built against, such as `1.8.3`.
    pub fn whisper_cpp_version() -> &'static str {
        whisper_rs::WHISPER_CPP_VERSION
    }

    /// whisper.cpp's description of the backends and CPU features compiled
    /// in. It names Metal when the Metal backend is part of the build; it
    /// does not say which backend a loaded model runs on.
    pub fn system_info() -> String {
        whisper_rs::print_system_info().to_owned()
    }
}

impl fmt::Debug for WhisperEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WhisperEngine")
            .field("model_path", &self.model_path)
            .field("multilingual", &self.multilingual)
            .field("threads", &self.threads)
            .finish_non_exhaustive()
    }
}

impl Transcriber for WhisperEngine {
    fn transcribe(
        &self,
        samples: &[f32],
        language: &Language,
    ) -> Result<Vec<TextSpan>, TranscribeError> {
        let whisper_language = resolve_language(self.multilingual, language)?;
        if samples.is_empty() {
            return Ok(Vec::new());
        }

        // Greedy decoding with whisper.cpp's default `best_of`.
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 5 });
        match whisper_language {
            WhisperLanguage::Detect => params.set_language(None),
            // `en` is whisper.cpp's default. Not setting it again matters:
            // whisper-rs 0.16.0 leaks the string on every `set_language`.
            WhisperLanguage::Code("en") => {}
            WhisperLanguage::Code(code) => params.set_language(Some(code)),
        }
        if let Some(threads) = self.threads {
            params.set_n_threads(i32::try_from(threads).unwrap_or(i32::MAX).max(1));
        }
        // Every slice stands alone: it may come from the other speaker or
        // follow a gap, so text from the previous call must not steer this
        // one.
        params.set_no_context(true);
        params.set_translate(false);
        // The transcript is returned, never printed.
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // A poisoned lock means an earlier call panicked. The state is reset
        // at the start of every run, so it is safe to keep using.
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .full(params, samples)
            .map_err(|error| TranscribeError::Engine {
                message: error.to_string(),
            })?;

        let slice_ms = duration_ms(samples.len());
        let mut spans = Vec::new();
        for segment in state.as_iter() {
            let text = segment
                .to_str_lossy()
                .map_err(|error| TranscribeError::Engine {
                    message: error.to_string(),
                })?;
            let Some(text) = clean_segment_text(&text) else {
                continue;
            };
            let start_ms = centiseconds_to_ms(segment.start_timestamp()).min(slice_ms);
            let end_ms = centiseconds_to_ms(segment.end_timestamp()).clamp(start_ms, slice_ms);
            spans.push(TextSpan {
                start_ms,
                end_ms,
                text,
            });
        }
        Ok(spans)
    }
}

/// Whisper reports segment times in hundredths of a second.
fn centiseconds_to_ms(centiseconds: i64) -> u64 {
    u64::try_from(centiseconds).unwrap_or(0).saturating_mul(10)
}

/// What to tell whisper.cpp about the language of a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhisperLanguage<'a> {
    /// Detect the language from the audio.
    Detect,
    /// A language code whisper.cpp knows.
    Code(&'a str),
}

/// Applies the language rules for a model.
///
/// - An English-only model takes `en`, and treats `auto` as `en`: it has no
///   language detection, and whisper.cpp would otherwise ignore the setting
///   without saying so. Any other language is an error.
/// - A multilingual model takes `auto` or any code in whisper.cpp's language
///   table.
fn resolve_language(
    multilingual: bool,
    language: &Language,
) -> Result<WhisperLanguage<'_>, TranscribeError> {
    match (multilingual, language.code()) {
        (false, None | Some("en")) => Ok(WhisperLanguage::Code("en")),
        (false, Some(other)) => Err(TranscribeError::EnglishOnlyModel {
            requested: other.to_owned(),
        }),
        (true, None) => Ok(WhisperLanguage::Detect),
        (true, Some(code)) => match whisper_rs::get_lang_id(code) {
            Some(_) => Ok(WhisperLanguage::Code(code)),
            None => Err(TranscribeError::UnsupportedLanguage {
                code: code.to_owned(),
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn language(text: &str) -> Language {
        text.parse().unwrap()
    }

    #[test]
    fn english_only_model_takes_english() {
        assert_eq!(
            resolve_language(false, &language("en")),
            Ok(WhisperLanguage::Code("en"))
        );
    }

    #[test]
    fn english_only_model_treats_auto_as_english() {
        assert_eq!(
            resolve_language(false, &Language::auto()),
            Ok(WhisperLanguage::Code("en"))
        );
    }

    #[test]
    fn english_only_model_rejects_other_languages() {
        for code in ["de", "fr", "ja", "haw", "zz"] {
            let error = resolve_language(false, &language(code)).unwrap_err();
            assert_eq!(
                error,
                TranscribeError::EnglishOnlyModel {
                    requested: code.to_owned()
                }
            );
            let message = error.to_string();
            assert!(message.contains("only transcribes English"), "{message}");
            assert!(message.contains(code), "{message}");
        }
    }

    #[test]
    fn multilingual_model_detects_on_auto() {
        assert_eq!(
            resolve_language(true, &Language::auto()),
            Ok(WhisperLanguage::Detect)
        );
    }

    #[test]
    fn multilingual_model_takes_known_codes() {
        for code in ["en", "de", "fr", "ja", "haw", "yue"] {
            assert_eq!(
                resolve_language(true, &language(code)),
                Ok(WhisperLanguage::Code(code)),
                "{code}"
            );
        }
    }

    #[test]
    fn multilingual_model_rejects_unknown_codes() {
        for code in ["zz", "xx", "qqq"] {
            assert_eq!(
                resolve_language(true, &language(code)),
                Err(TranscribeError::UnsupportedLanguage {
                    code: code.to_owned()
                }),
                "{code}"
            );
        }
    }

    #[test]
    fn segment_times_convert_to_milliseconds() {
        assert_eq!(centiseconds_to_ms(0), 0);
        assert_eq!(centiseconds_to_ms(1), 10);
        assert_eq!(centiseconds_to_ms(575), 5_750);
        assert_eq!(centiseconds_to_ms(-3), 0);
        assert_eq!(centiseconds_to_ms(i64::MAX), u64::MAX);
    }

    #[test]
    fn missing_model_file_is_reported_with_its_path() {
        let path = Path::new("/nonexistent/mneme/ggml-missing.bin");
        let error = WhisperEngine::load(path).unwrap_err();
        assert_eq!(
            error,
            TranscribeError::ModelNotFound {
                path: path.to_path_buf()
            }
        );
        assert!(error.to_string().contains("ggml-missing.bin"));
    }

    #[test]
    fn build_reports_its_whisper_cpp_version() {
        let version = WhisperEngine::whisper_cpp_version();
        assert!(
            version.split('.').count() == 3
                && version.split('.').all(|part| part.parse::<u32>().is_ok()),
            "unexpected version {version:?}"
        );
    }

    #[test]
    fn metal_backend_is_compiled_in() {
        let info = WhisperEngine::system_info();
        assert!(info.contains("Metal"), "no Metal in system info: {info}");
    }
}

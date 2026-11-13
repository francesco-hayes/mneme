//! The sample formats a tap can deliver, read from its
//! `AudioStreamBasicDescription`.
//!
//! In practice a tap delivers native-endian 32-bit float, interleaved, at the
//! rate of the output device (that is what Apple's sample, AudioCap and
//! Chromium all observe). Nothing in the API promises it, so the description
//! is parsed rather than assumed, and the other linear PCM layouts Core Audio
//! uses are accepted too. Anything else is refused with
//! [`AudioError::UnsupportedFormat`] instead of being misread as noise.

use objc2_core_audio_types::{
    AudioStreamBasicDescription, kAudioFormatFlagIsBigEndian, kAudioFormatFlagIsFloat,
    kAudioFormatFlagIsNonInterleaved, kAudioFormatFlagIsSignedInteger, kAudioFormatLinearPCM,
};

use crate::AudioError;
use crate::convert::{ToF32, i24_to_f32};

/// Highest sample rate accepted from a device. Far above any real one; it
/// only keeps a corrupt description from reaching the resampler.
const MAX_SAMPLE_RATE: f64 = 768_000.0;

/// How one sample is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SampleKind {
    F32,
    F64,
    I16,
    /// 24-bit signed, packed in three bytes.
    I24,
    I32,
}

/// What a tap delivers, as far as conversion needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TapFormat {
    /// Frames per second, rounded to a whole number.
    pub sample_rate: u32,
    pub channels: u32,
    pub kind: SampleKind,
    /// `true`: one buffer holding all channels frame by frame. `false`: one
    /// buffer per channel.
    pub interleaved: bool,
}

impl TapFormat {
    /// Reads a stream description, refusing every layout the IO proc could
    /// not convert.
    pub(crate) fn from_description(
        description: &AudioStreamBasicDescription,
    ) -> Result<Self, AudioError> {
        let unsupported = |what: String| Err(AudioError::UnsupportedFormat(what));

        if description.mFormatID != kAudioFormatLinearPCM {
            return unsupported(format!(
                "the tap delivers format {}, not linear PCM",
                four_char_code(description.mFormatID)
            ));
        }
        let flags = description.mFormatFlags;
        // Every Mac this runs on is little-endian, so "big-endian" means
        // "not native".
        if flags & kAudioFormatFlagIsBigEndian != 0 {
            return unsupported("the tap delivers big-endian samples".to_owned());
        }
        let channels = description.mChannelsPerFrame;
        if channels == 0 {
            return unsupported("the tap delivers zero channels".to_owned());
        }
        let sample_rate = sample_rate_hz(description.mSampleRate)?;

        let interleaved = flags & kAudioFormatFlagIsNonInterleaved == 0;
        // A frame covers every channel when interleaved, one otherwise.
        let per_frame = if interleaved { channels } else { 1 };
        if description.mBytesPerFrame == 0 || !description.mBytesPerFrame.is_multiple_of(per_frame)
        {
            return unsupported(format!(
                "the tap delivers {} bytes per frame for {channels} channel(s)",
                description.mBytesPerFrame
            ));
        }
        let bytes = description.mBytesPerFrame / per_frame;
        let bits = description.mBitsPerChannel;

        let kind = if flags & kAudioFormatFlagIsFloat != 0 {
            match (bits, bytes) {
                (32, 4) => SampleKind::F32,
                (64, 8) => SampleKind::F64,
                _ => {
                    return unsupported(format!(
                        "the tap delivers {bits}-bit float samples in {bytes} bytes"
                    ));
                }
            }
        } else if flags & kAudioFormatFlagIsSignedInteger != 0 {
            match (bits, bytes) {
                (16, 2) => SampleKind::I16,
                (24, 3) => SampleKind::I24,
                (32, 4) => SampleKind::I32,
                _ => {
                    return unsupported(format!(
                        "the tap delivers {bits}-bit integer samples in {bytes} bytes"
                    ));
                }
            }
        } else {
            return unsupported(format!(
                "the tap delivers unsigned {bits}-bit integer samples"
            ));
        };

        Ok(Self {
            sample_rate,
            channels,
            kind,
            interleaved,
        })
    }
}

/// Checks a sample rate reported by Core Audio and rounds it to whole hertz.
pub(crate) fn sample_rate_hz(rate: f64) -> Result<u32, AudioError> {
    if rate.is_finite() && (1.0..=MAX_SAMPLE_RATE).contains(&rate) {
        Ok(rate.round() as u32)
    } else {
        Err(AudioError::UnsupportedFormat(format!(
            "the tap reports a sample rate of {rate} Hz"
        )))
    }
}

/// Renders a Core Audio four-character code (format IDs and most error
/// statuses are four ASCII characters packed into a number): `'lpcm'`,
/// `'!obj'`. A number that is not one is rendered as the number.
pub(crate) fn four_char_code(code: u32) -> String {
    let bytes = code.to_be_bytes();
    if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        let text: String = bytes.iter().map(|b| char::from(*b)).collect();
        format!("'{text}'")
    } else {
        // Statuses that are plain numbers are negative.
        (code as i32).to_string()
    }
}

// Sample types as raw bytes. Their alignment is 1, so a slice of them can be
// laid over any buffer Core Audio hands out, whatever its address, and they
// run through the same `ToF32` downmix code as every other source. Each
// conversion compiles to a plain load.

macro_rules! byte_sample {
    ($(#[$doc:meta])* $name:ident, $bytes:literal, |$raw:ident| $convert:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(transparent)]
        pub(crate) struct $name(pub [u8; $bytes]);

        impl ToF32 for $name {
            #[inline]
            fn to_f32(self) -> f32 {
                let $raw = self.0;
                $convert
            }
        }
    };
}

byte_sample!(
    /// Native-endian `f32`.
    F32Sample,
    4,
    |raw| f32::from_ne_bytes(raw)
);
byte_sample!(
    /// Native-endian `f64`.
    F64Sample,
    8,
    |raw| f64::from_ne_bytes(raw).to_f32()
);
byte_sample!(
    /// Native-endian `i16`.
    I16Sample,
    2,
    |raw| i16::from_ne_bytes(raw).to_f32()
);
byte_sample!(
    /// Native-endian `i32`.
    I32Sample,
    4,
    |raw| i32::from_ne_bytes(raw).to_f32()
);
byte_sample!(
    /// Little-endian packed signed 24-bit.
    I24Sample,
    3,
    |raw| {
        // Put the three bytes at the top of an i32, then shift back down so
        // the sign bit is carried.
        i24_to_f32(i32::from_le_bytes([0, raw[0], raw[1], raw[2]]) >> 8)
    }
);

#[cfg(test)]
pub(crate) mod tests {
    use objc2_core_audio_types::{kAudioFormatFlagIsPacked, kAudioFormatFlagsNativeFloatPacked};

    use super::*;

    /// A linear PCM description with the given layout.
    pub(crate) fn description(
        rate: f64,
        channels: u32,
        flags: u32,
        bits: u32,
        interleaved: bool,
    ) -> AudioStreamBasicDescription {
        let per_frame = if interleaved { channels } else { 1 };
        let bytes_per_frame = bits / 8 * per_frame;
        AudioStreamBasicDescription {
            mSampleRate: rate,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: if interleaved {
                flags
            } else {
                flags | kAudioFormatFlagIsNonInterleaved
            },
            mBytesPerPacket: bytes_per_frame,
            mFramesPerPacket: 1,
            mBytesPerFrame: bytes_per_frame,
            mChannelsPerFrame: channels,
            mBitsPerChannel: bits,
            mReserved: 0,
        }
    }

    const FLOAT: u32 = kAudioFormatFlagsNativeFloatPacked;
    const INT: u32 = kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked;

    #[test]
    fn the_format_taps_deliver_is_read() {
        // 48 kHz interleaved stereo float: what a stereo mixdown tap reports.
        let format =
            TapFormat::from_description(&description(48_000.0, 2, FLOAT, 32, true)).unwrap();
        assert_eq!(
            format,
            TapFormat {
                sample_rate: 48_000,
                channels: 2,
                kind: SampleKind::F32,
                interleaved: true,
            }
        );
    }

    #[test]
    fn other_rates_and_channel_counts_are_read() {
        for rate in [
            8_000.0, 16_000.0, 24_000.0, 44_100.0, 88_200.0, 96_000.0, 192_000.0,
        ] {
            for channels in [1, 2, 6] {
                let format =
                    TapFormat::from_description(&description(rate, channels, FLOAT, 32, true))
                        .unwrap();
                assert_eq!(format.sample_rate, rate as u32);
                assert_eq!(format.channels, channels);
            }
        }
    }

    #[test]
    fn non_interleaved_and_integer_layouts_are_read() {
        let planar =
            TapFormat::from_description(&description(44_100.0, 2, FLOAT, 32, false)).unwrap();
        assert!(!planar.interleaved);
        assert_eq!(planar.kind, SampleKind::F32);
        // Non-interleaved: a "frame" is one channel's sample.
        assert_eq!(description(44_100.0, 2, FLOAT, 32, false).mBytesPerFrame, 4);

        let cases = [
            (FLOAT, 64, SampleKind::F64),
            (INT, 16, SampleKind::I16),
            (INT, 24, SampleKind::I24),
            (INT, 32, SampleKind::I32),
        ];
        for (flags, bits, kind) in cases {
            for interleaved in [true, false] {
                let format = TapFormat::from_description(&description(
                    48_000.0,
                    2,
                    flags,
                    bits,
                    interleaved,
                ))
                .unwrap();
                assert_eq!(format.kind, kind);
                assert_eq!(format.interleaved, interleaved);
            }
        }
    }

    #[test]
    fn layouts_that_cannot_be_converted_are_refused() {
        let unsupported = |d: AudioStreamBasicDescription| {
            matches!(
                TapFormat::from_description(&d),
                Err(AudioError::UnsupportedFormat(_))
            )
        };
        let good = description(48_000.0, 2, FLOAT, 32, true);
        assert!(!unsupported(good));

        // Not linear PCM ('aac ').
        assert!(unsupported(AudioStreamBasicDescription {
            mFormatID: 0x6161_6320,
            ..good
        }));
        assert!(unsupported(AudioStreamBasicDescription {
            mFormatFlags: FLOAT | kAudioFormatFlagIsBigEndian,
            ..good
        }));
        assert!(unsupported(AudioStreamBasicDescription {
            mChannelsPerFrame: 0,
            ..good
        }));
        assert!(unsupported(AudioStreamBasicDescription {
            mBytesPerFrame: 0,
            ..good
        }));
        // Frame size that is not a whole number of samples per channel.
        assert!(unsupported(AudioStreamBasicDescription {
            mBytesPerFrame: 7,
            ..good
        }));
        // 24 bits in a 4-byte container, 16-bit float, unsigned integers.
        assert!(unsupported(AudioStreamBasicDescription {
            mBitsPerChannel: 24,
            ..description(48_000.0, 2, INT, 32, true)
        }));
        assert!(unsupported(description(48_000.0, 2, FLOAT, 16, true)));
        assert!(unsupported(description(
            48_000.0,
            2,
            kAudioFormatFlagIsPacked,
            8,
            true
        )));
        for rate in [0.0, -44_100.0, f64::NAN, f64::INFINITY, 1.0e9] {
            assert!(unsupported(description(rate, 2, FLOAT, 32, true)), "{rate}");
        }
    }

    #[test]
    fn fractional_rates_round_to_whole_hertz() {
        assert_eq!(sample_rate_hz(44_099.6).unwrap(), 44_100);
        assert_eq!(sample_rate_hz(48_000.0).unwrap(), 48_000);
    }

    #[test]
    fn byte_samples_convert_like_their_typed_counterparts() {
        assert_eq!(F32Sample(0.25f32.to_ne_bytes()).to_f32(), 0.25);
        assert_eq!(F64Sample((-0.5f64).to_ne_bytes()).to_f32(), -0.5);
        assert_eq!(I16Sample(i16::MIN.to_ne_bytes()).to_f32(), -1.0);
        assert_eq!(I16Sample(16_384i16.to_ne_bytes()).to_f32(), 0.5);
        assert_eq!(I32Sample(i32::MIN.to_ne_bytes()).to_f32(), -1.0);
        assert_eq!(I32Sample((1i32 << 30).to_ne_bytes()).to_f32(), 0.5);

        let i24 = |value: i32| {
            let bytes = value.to_le_bytes();
            I24Sample([bytes[0], bytes[1], bytes[2]]).to_f32()
        };
        assert_eq!(i24(-8_388_608), -1.0);
        assert_eq!(i24(4_194_304), 0.5);
        assert_eq!(i24(-4_194_304), -0.5);
        assert_eq!(i24(0), 0.0);
        assert_eq!(i24(-1), -1.0 / 8_388_608.0);
    }

    #[test]
    fn byte_samples_have_no_alignment_requirement() {
        assert_eq!(std::mem::align_of::<F32Sample>(), 1);
        assert_eq!(std::mem::align_of::<F64Sample>(), 1);
        assert_eq!(std::mem::align_of::<I16Sample>(), 1);
        assert_eq!(std::mem::align_of::<I24Sample>(), 1);
        assert_eq!(std::mem::align_of::<I32Sample>(), 1);
        assert_eq!(std::mem::size_of::<I24Sample>(), 3);
        assert_eq!(std::mem::size_of::<F64Sample>(), 8);
    }

    #[test]
    fn four_char_codes_are_readable() {
        assert_eq!(four_char_code(kAudioFormatLinearPCM), "'lpcm'");
        assert_eq!(four_char_code(0x2168_6f67), "'!hog'");
        assert_eq!(four_char_code(7), "7");
        assert_eq!(four_char_code(-50i32 as u32), "-50");
    }
}

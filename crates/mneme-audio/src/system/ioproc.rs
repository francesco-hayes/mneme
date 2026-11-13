//! The IO proc: the function Core Audio calls on its real-time thread with
//! each buffer the tap captured.
//!
//! It does what the microphone's callback does and no more: find the samples
//! in the `AudioBufferList`, downmix them to mono through the shared
//! conversion code, work out when the first sample was captured, and hand
//! the buffer to the capture thread. Resampling and framing happen there.
//!
//! The buffer list is taken as it comes rather than as the tap's format says
//! it should be: one buffer holding interleaved channels, or one buffer per
//! channel. Chromium notes that the channel count of a Bluetooth device can
//! change under a running tap, so each callback goes by the counts in its
//! own buffers.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use objc2_core_audio::{AudioConvertHostTimeToNanos, AudioGetCurrentHostTime, AudioObjectID};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp, AudioTimeStampFlags};

use super::format::{F32Sample, F64Sample, I16Sample, I24Sample, I32Sample, SampleKind};
use crate::capture::StreamEvents;
use crate::convert::ToF32;

/// A host timestamp further in the past than this is not believed.
const MAX_CAPTURE_AGE: Duration = Duration::from_secs(1);

/// The IO proc's signature with plain pointers. Core Audio's headers declare
/// every pointer non-null; taking raw pointers and checking them costs
/// nothing and does not rest on that promise.
pub(crate) type RawIoProc = unsafe extern "C-unwind" fn(
    AudioObjectID,
    *const AudioTimeStamp,
    *const AudioBufferList,
    *const AudioTimeStamp,
    *mut AudioBufferList,
    *const AudioTimeStamp,
    *mut c_void,
) -> i32;

/// What the IO proc needs, registered with it as client data. Owned by the
/// [`TapSession`](super::session::TapSession), which frees it only once Core
/// Audio can no longer call the proc.
pub(crate) struct IoContext {
    /// Cleared before teardown starts, so a callback that is still delivered
    /// does nothing.
    active: AtomicBool,
    events: StreamEvents,
    kind: SampleKind,
    sample_rate: u32,
}

impl IoContext {
    pub(crate) fn new(events: StreamEvents, kind: SampleKind, sample_rate: u32) -> Self {
        Self {
            active: AtomicBool::new(true),
            events,
            kind,
            sample_rate,
        }
    }

    /// Makes every later callback a no-op.
    pub(crate) fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
    }

    /// Converts one callback's buffers and sends them to the capture thread.
    ///
    /// `age` is how long ago the first sample was captured according to the
    /// callback's host timestamp, if it carried a usable one.
    ///
    /// # Safety
    ///
    /// Every buffer's `mData` must be null or point to `mDataByteSize`
    /// readable bytes that stay valid for the duration of the call.
    pub(crate) unsafe fn deliver(
        &self,
        buffers: &[AudioBuffer],
        now: Instant,
        age: Option<Duration>,
    ) {
        // SAFETY: forwarded from the caller.
        unsafe {
            match self.kind {
                SampleKind::F32 => self.deliver_as::<F32Sample>(buffers, now, age),
                SampleKind::F64 => self.deliver_as::<F64Sample>(buffers, now, age),
                SampleKind::I16 => self.deliver_as::<I16Sample>(buffers, now, age),
                SampleKind::I24 => self.deliver_as::<I24Sample>(buffers, now, age),
                SampleKind::I32 => self.deliver_as::<I32Sample>(buffers, now, age),
            }
        }
    }

    /// # Safety
    ///
    /// As [`deliver`](Self::deliver). `T` must have alignment 1 and be valid
    /// for every bit pattern, which holds for the byte sample types.
    unsafe fn deliver_as<T: ToF32>(
        &self,
        buffers: &[AudioBuffer],
        now: Instant,
        age: Option<Duration>,
    ) {
        let usable = |buffer: &&AudioBuffer| {
            !buffer.mData.is_null()
                && buffer.mNumberChannels > 0
                && buffer.mDataByteSize as usize >= size_of::<T>() * buffer.mNumberChannels as usize
        };
        // SAFETY: `usable` checked the pointer; the caller vouches for the
        // length; `T` has alignment 1.
        let samples = |buffer: &AudioBuffer| unsafe {
            std::slice::from_raw_parts(
                buffer.mData.cast::<T>().cast_const(),
                buffer.mDataByteSize as usize / size_of::<T>(),
            )
        };

        let mut found = buffers.iter().filter(usable);
        let Some(first) = found.next() else {
            return;
        };
        let more = found.next().is_some();
        let all_mono = buffers
            .iter()
            .filter(usable)
            .all(|buffer| buffer.mNumberChannels == 1);

        if more && all_mono {
            // Non-interleaved: one buffer per channel.
            let channels: Vec<&[T]> = buffers.iter().filter(usable).map(samples).collect();
            let frames = channels.iter().map(|c| c.len()).min().unwrap_or(0);
            let at = first_sample_instant(now, age, frames, self.sample_rate);
            self.events.data_planar(&channels, at);
        } else {
            // Interleaved. A device with several multi-channel streams would
            // deliver more buffers; like Chromium, only the first is used.
            let channels = first.mNumberChannels as usize;
            let interleaved = samples(first);
            let at = first_sample_instant(now, age, interleaved.len() / channels, self.sample_rate);
            self.events.data(interleaved, channels, at);
        }
    }
}

/// When the first sample of a buffer was captured.
///
/// The IO proc's input timestamp says when the first frame was acquired; its
/// distance from the present is `age`. When there is no usable timestamp
/// (none given, in the future, or implausibly old) the buffer's own duration
/// is used: the first sample cannot be younger than that.
///
/// This instant only places a new stream on the meeting clock and reveals a
/// stall. Between those, the [`Framer`](crate::Framer) counts samples.
pub(crate) fn first_sample_instant(
    now: Instant,
    age: Option<Duration>,
    frames: usize,
    sample_rate: u32,
) -> Instant {
    let delay = match age {
        Some(age) if age <= MAX_CAPTURE_AGE => age,
        _ => Duration::from_micros(frames as u64 * 1_000_000 / u64::from(sample_rate.max(1))),
    };
    now.checked_sub(delay).unwrap_or(now)
}

/// How long ago `host_time` (in host clock ticks, as in
/// `AudioTimeStamp::mHostTime`) was, or `None` if it lies in the future.
pub(crate) fn host_time_age(host_time: u64) -> Option<Duration> {
    // SAFETY: both functions only read the host clock and its fixed
    // timebase; they take no pointers.
    let ticks = unsafe { AudioGetCurrentHostTime() }.checked_sub(host_time)?;
    Some(Duration::from_nanos(unsafe {
        AudioConvertHostTimeToNanos(ticks)
    }))
}

/// The function registered with `AudioDeviceCreateIOProcID`.
///
/// # Safety
///
/// Called by Core Audio only. `client_data` must be null or the
/// [`IoContext`] registered with the proc, still alive; `input` and
/// `input_time` must be null or valid for the duration of the call.
pub(crate) unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: *const AudioTimeStamp,
    input: *const AudioBufferList,
    input_time: *const AudioTimeStamp,
    _output: *mut AudioBufferList,
    _output_time: *const AudioTimeStamp,
    client_data: *mut c_void,
) -> i32 {
    // A panic must not unwind into Core Audio's IO thread.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: forwarded from the caller.
        unsafe { on_io(input, input_time, client_data) }
    }));
    // "The return value is currently unused and should always be 0."
    0
}

/// # Safety
///
/// As [`io_proc`].
unsafe fn on_io(
    input: *const AudioBufferList,
    input_time: *const AudioTimeStamp,
    client_data: *mut c_void,
) {
    // SAFETY: null or the live context, per the caller.
    let Some(context) = (unsafe { client_data.cast::<IoContext>().cast_const().as_ref() }) else {
        return;
    };
    if !context.active.load(Ordering::Acquire) || input.is_null() {
        return;
    }
    let now = Instant::now();
    // SAFETY: null or valid, per the caller.
    let age = unsafe { input_time.as_ref() }
        .filter(|time| time.mFlags.contains(AudioTimeStampFlags::HostTimeValid))
        .and_then(|time| host_time_age(time.mHostTime));

    // SAFETY: `AudioBufferList` is a count followed by that many buffers;
    // `mBuffers` is declared with one element and continues past it.
    let buffers = unsafe {
        std::slice::from_raw_parts(
            (&raw const (*input).mBuffers).cast::<AudioBuffer>(),
            (*input).mNumberBuffers as usize,
        )
    };
    // SAFETY: Core Audio keeps the buffers valid during the callback.
    unsafe { context.deliver(buffers, now, age) };
}

#[cfg(test)]
pub(crate) mod tests {
    use mneme_core::{AudioFrame, Speaker};
    use objc2_core_audio::AudioConvertNanosToHostTime;
    use objc2_core_audio_types::{SMPTETime, SMPTETimeFlags, SMPTETimeType};

    use super::*;
    use crate::capture::tests::logging_events;
    use crate::convert::{downmix_to_mono, to_16k_mono};
    use crate::testutil::{fit_sine, sine, zero_crossing_frequency};
    use crate::{Framer, frame_channel};

    /// An `AudioBufferList` with room for `N` buffers, laid out as Core
    /// Audio lays one out.
    #[repr(C)]
    pub(crate) struct BufferList<const N: usize> {
        count: u32,
        buffers: [AudioBuffer; N],
    }

    impl<const N: usize> BufferList<N> {
        /// Describes `N` byte buffers, each holding `channels` channels.
        /// The list borrows the bytes: keep them alive while it is used.
        pub(crate) fn over(buffers: [&[u8]; N], channels: u32) -> Self {
            Self {
                count: N as u32,
                buffers: buffers.map(|bytes| AudioBuffer {
                    mNumberChannels: channels,
                    mDataByteSize: bytes.len() as u32,
                    mData: bytes.as_ptr().cast_mut().cast(),
                }),
            }
        }

        pub(crate) fn as_ptr(&self) -> *const AudioBufferList {
            std::ptr::from_ref(self).cast()
        }
    }

    /// A timestamp for a buffer whose first sample was captured `ago` ago.
    pub(crate) fn timestamp(ago: Duration) -> AudioTimeStamp {
        // SAFETY: pure clock functions.
        let host = unsafe {
            AudioGetCurrentHostTime() - AudioConvertNanosToHostTime(ago.as_nanos() as u64)
        };
        AudioTimeStamp {
            mSampleTime: 0.0,
            mHostTime: host,
            mRateScalar: 1.0,
            mWordClockTime: 0,
            mSMPTETime: SMPTETime {
                mSubframes: 0,
                mSubframeDivisor: 0,
                mCounter: 0,
                mType: SMPTETimeType(0),
                mFlags: SMPTETimeFlags(0),
                mHours: 0,
                mMinutes: 0,
                mSeconds: 0,
                mFrames: 0,
            },
            mFlags: AudioTimeStampFlags::HostTimeValid,
            mReserved: 0,
        }
    }

    /// Calls the real IO proc the way Core Audio does.
    pub(crate) fn call_io_proc(
        context: *const IoContext,
        list: *const AudioBufferList,
        time: &AudioTimeStamp,
    ) {
        // SAFETY: the context and list are alive for the call; the output
        // arguments are never read.
        let status = unsafe {
            io_proc(
                0,
                time,
                list,
                time,
                std::ptr::null_mut(),
                std::ptr::null(),
                context.cast_mut().cast(),
            )
        };
        assert_eq!(status, 0);
    }

    pub(crate) fn bytes_of<T: Copy, const N: usize>(
        samples: &[T],
        to_bytes: impl Fn(T) -> [u8; N],
    ) -> Vec<u8> {
        samples.iter().flat_map(|s| to_bytes(*s)).collect()
    }

    fn i24_bytes(sample: f32) -> [u8; 3] {
        let value = (sample * 8_388_607.0).round() as i32;
        let bytes = value.to_le_bytes();
        [bytes[0], bytes[1], bytes[2]]
    }

    /// Runs one buffer list through the IO proc and returns the mono buffers
    /// that reached the capture thread's side of the channel.
    fn through_io_proc<const N: usize>(
        kind: SampleKind,
        rate: u32,
        buffers: [&[u8]; N],
        channels: u32,
    ) -> Vec<Vec<f32>> {
        let (events, log) = logging_events();
        let context = IoContext::new(events, kind, rate);
        let list = BufferList::over(buffers, channels);
        call_io_proc(&context, list.as_ptr(), &timestamp(Duration::ZERO));
        drop(context);
        log.data().into_iter().map(|(mono, _)| mono).collect()
    }

    fn assert_close(actual: &[f32], expected: &[f32], tolerance: f32) {
        assert_eq!(actual.len(), expected.len());
        for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!((a - e).abs() <= tolerance, "sample {i}: {a} vs {e}");
        }
    }

    #[test]
    fn interleaved_float_stereo_is_downmixed() {
        // What a stereo mixdown tap delivers: one buffer, L R L R.
        let left = sine(440.0, 48_000, 480);
        let right = sine(880.0, 48_000, 480);
        let stereo: Vec<f32> = left
            .iter()
            .zip(&right)
            .flat_map(|(l, r)| [*l, *r])
            .collect();
        let bytes = bytes_of(&stereo, f32::to_ne_bytes);

        let got = through_io_proc(SampleKind::F32, 48_000, [&bytes], 2);
        assert_eq!(got.len(), 1);
        // Exactly what the shared conversion code makes of the same samples.
        assert_eq!(got[0], downmix_to_mono(&stereo, 2));
        let expected: Vec<f32> = left
            .iter()
            .zip(&right)
            .map(|(l, r)| (l + r) * 0.5)
            .collect();
        assert_close(&got[0], &expected, 1e-6);
    }

    #[test]
    fn planar_float_stereo_is_downmixed() {
        // Non-interleaved: one mono buffer per channel.
        let left = sine(440.0, 44_100, 441);
        let right: Vec<f32> = left.iter().map(|s| -0.5 * s).collect();
        let (l, r) = (
            bytes_of(&left, f32::to_ne_bytes),
            bytes_of(&right, f32::to_ne_bytes),
        );

        let got = through_io_proc(SampleKind::F32, 44_100, [&l, &r], 1);
        assert_eq!(got.len(), 1);
        let expected: Vec<f32> = left.iter().map(|s| s * 0.25).collect();
        assert_close(&got[0], &expected, 1e-6);
    }

    #[test]
    fn mono_and_multichannel_buffers_are_handled() {
        let mono = sine(300.0, 16_000, 160);
        let got = through_io_proc(
            SampleKind::F32,
            16_000,
            [&bytes_of(&mono, f32::to_ne_bytes)],
            1,
        );
        assert_eq!(got, std::slice::from_ref(&mono));

        // 5.1: six interleaved channels, the tone on one of them.
        let surround: Vec<f32> = mono
            .iter()
            .flat_map(|s| [*s, 0.0, 0.0, 0.0, 0.0, 0.0])
            .collect();
        let got = through_io_proc(
            SampleKind::F32,
            16_000,
            [&bytes_of(&surround, f32::to_ne_bytes)],
            6,
        );
        let expected: Vec<f32> = mono.iter().map(|s| s / 6.0).collect();
        assert_close(&got[0], &expected, 1e-6);
    }

    #[test]
    fn integer_and_double_formats_are_converted() {
        let tone = sine(1_000.0, 48_000, 480);
        let stereo: Vec<f32> = tone.iter().flat_map(|s| [*s, *s]).collect();

        let f64s = bytes_of(&stereo, |s| f64::from(s).to_ne_bytes());
        let got = through_io_proc(SampleKind::F64, 48_000, [&f64s], 2);
        assert_close(&got[0], &tone, 1e-6);

        let i16s = bytes_of(&stereo, |s| ((s * 32_767.0).round() as i16).to_ne_bytes());
        let got = through_io_proc(SampleKind::I16, 48_000, [&i16s], 2);
        assert_close(&got[0], &tone, 1.0 / 16_000.0);

        let i24s = bytes_of(&stereo, i24_bytes);
        let got = through_io_proc(SampleKind::I24, 48_000, [&i24s], 2);
        assert_close(&got[0], &tone, 1.0 / 4_000_000.0);

        let i32s = bytes_of(&stereo, |s| {
            ((f64::from(s) * 2_147_483_647.0).round() as i32).to_ne_bytes()
        });
        let got = through_io_proc(SampleKind::I32, 48_000, [&i32s], 2);
        assert_close(&got[0], &tone, 1e-6);

        // Planar integers too.
        let half = bytes_of(&tone, |s| ((s * 16_383.0).round() as i16).to_ne_bytes());
        let full = bytes_of(&tone, |s| ((s * 32_767.0).round() as i16).to_ne_bytes());
        let got = through_io_proc(SampleKind::I16, 48_000, [&full, &half], 1);
        let expected: Vec<f32> = tone.iter().map(|s| s * 0.75).collect();
        assert_close(&got[0], &expected, 1.0 / 16_000.0);
    }

    #[test]
    fn buffers_at_odd_addresses_are_read_correctly() {
        // Core Audio aligns its buffers; nothing here depends on it.
        let tone = sine(500.0, 48_000, 96);
        let mut bytes = vec![0u8];
        bytes.extend(bytes_of(&tone, f32::to_ne_bytes));
        let got = through_io_proc(SampleKind::F32, 48_000, [&bytes[1..]], 1);
        assert_eq!(got, [tone]);
    }

    #[test]
    fn empty_disabled_and_ragged_buffers_do_not_break_the_callback() {
        let tone = sine(500.0, 48_000, 96);
        let bytes = bytes_of(&tone, f32::to_ne_bytes);

        // No buffers at all, and a null list.
        assert!(through_io_proc::<0>(SampleKind::F32, 48_000, [], 2).is_empty());
        let (events, log) = logging_events();
        let context = IoContext::new(events, SampleKind::F32, 48_000);
        call_io_proc(&context, std::ptr::null(), &timestamp(Duration::ZERO));
        // No client data.
        let list = BufferList::over([&bytes[..]], 1);
        call_io_proc(std::ptr::null(), list.as_ptr(), &timestamp(Duration::ZERO));
        assert!(log.data().is_empty());

        // A disabled stream: null data but a byte size ("how much data would
        // have been there"). The enabled stream after it is still used.
        let mut list = BufferList::over([&bytes[..], &bytes[..]], 1);
        list.buffers[0].mData = std::ptr::null_mut();
        call_io_proc(&context, list.as_ptr(), &timestamp(Duration::ZERO));
        assert_eq!(log.data().len(), 1);
        assert_eq!(log.data()[0].0, tone);

        // Zero channels, empty data, and a size that is not whole frames.
        let got = through_io_proc(SampleKind::F32, 48_000, [&bytes], 0);
        assert!(got.is_empty());
        let got = through_io_proc(SampleKind::F32, 48_000, [&bytes[..0]], 2);
        assert!(got.is_empty());
        let got = through_io_proc(SampleKind::F32, 48_000, [&bytes[..4 * 5 + 3]], 2);
        assert_eq!(got[0].len(), 2, "two whole stereo frames out of 23 bytes");

        // Planar channels of different lengths: the shortest decides.
        let got = through_io_proc(SampleKind::F32, 48_000, [&bytes[..40], &bytes[..80]], 1);
        assert_eq!(got[0].len(), 10);
    }

    #[test]
    fn a_deactivated_context_delivers_nothing() {
        let bytes = bytes_of(&sine(500.0, 48_000, 96), f32::to_ne_bytes);
        let (events, log) = logging_events();
        let context = IoContext::new(events, SampleKind::F32, 48_000);
        let list = BufferList::over([&bytes[..]], 1);
        call_io_proc(&context, list.as_ptr(), &timestamp(Duration::ZERO));
        context.deactivate();
        call_io_proc(&context, list.as_ptr(), &timestamp(Duration::ZERO));
        assert_eq!(log.data().len(), 1);
    }

    #[test]
    fn capture_time_comes_from_the_host_timestamp() {
        let now = Instant::now();
        let ms = Duration::from_millis;
        // A plausible timestamp is used as is.
        assert_eq!(
            first_sample_instant(now, Some(ms(12)), 512, 48_000),
            now - ms(12)
        );
        assert_eq!(first_sample_instant(now, Some(ms(0)), 512, 48_000), now);
        assert_eq!(
            first_sample_instant(now, Some(ms(1_000)), 512, 48_000),
            now - ms(1_000)
        );
        // None, or one too old to believe: the buffer's duration (10 ms).
        assert_eq!(first_sample_instant(now, None, 480, 48_000), now - ms(10));
        assert_eq!(
            first_sample_instant(now, Some(ms(1_001)), 480, 48_000),
            now - ms(10)
        );
        assert_eq!(
            first_sample_instant(now, Some(Duration::from_secs(86_400)), 441, 44_100),
            now - ms(10)
        );
        // A zero rate cannot divide by zero.
        let _ = first_sample_instant(now, None, 480, 0);
    }

    #[test]
    fn host_time_is_converted_to_an_age() {
        // SAFETY: pure clock functions.
        let (now, forty_ms) = unsafe {
            (
                AudioGetCurrentHostTime(),
                AudioConvertNanosToHostTime(40_000_000),
            )
        };
        let age = host_time_age(now - forty_ms).unwrap();
        assert!(age >= Duration::from_millis(40), "{age:?}");
        // Loose on purpose: this thread may be kept waiting on a busy machine.
        assert!(age < Duration::from_secs(5), "{age:?}");
        // Host ticks convert to time at the right scale.
        let one_second = unsafe { AudioConvertNanosToHostTime(1_000_000_000) };
        assert_eq!(
            unsafe { AudioConvertHostTimeToNanos(one_second) }.div_ceil(1_000_000),
            1_000
        );
        // A timestamp in the future has no age.
        assert_eq!(host_time_age(now + 100 * forty_ms), None);
    }

    #[test]
    fn the_io_proc_stamps_buffers_with_their_capture_time() {
        let bytes = bytes_of(&sine(500.0, 48_000, 480), f32::to_ne_bytes);
        let list = BufferList::over([&bytes[..]], 1);
        let (events, log) = logging_events();
        let context = IoContext::new(events, SampleKind::F32, 48_000);

        // Captured 200 ms ago according to the host clock.
        let before = Instant::now();
        call_io_proc(
            &context,
            list.as_ptr(),
            &timestamp(Duration::from_millis(200)),
        );
        let after = Instant::now();
        let at = log.data()[0].1;
        assert!(at >= before - Duration::from_millis(260), "too early");
        assert!(at <= after - Duration::from_millis(200), "too late");

        // No valid host time: the buffer's duration (10 ms) is assumed.
        let mut invalid = timestamp(Duration::from_millis(200));
        invalid.mFlags = AudioTimeStampFlags(0);
        let before = Instant::now();
        call_io_proc(&context, list.as_ptr(), &invalid);
        let after = Instant::now();
        let at = log.data()[1].1;
        assert!(at >= before - Duration::from_millis(10));
        assert!(at <= after - Duration::from_millis(10));
    }

    /// The whole path a tap buffer takes, with no device: IO proc, shared
    /// downmix, resampler, framer. One second of a 1 kHz tone as 10 ms
    /// callbacks in each layout a tap may use.
    #[test]
    fn tap_buffers_become_16k_mono_frames() {
        struct Case {
            rate: u32,
            kind: SampleKind,
            channels: usize,
            interleaved: bool,
        }
        let case = |rate, kind, channels, interleaved| Case {
            rate,
            kind,
            channels,
            interleaved,
        };
        let cases = [
            case(48_000, SampleKind::F32, 2, true),
            case(44_100, SampleKind::F32, 2, true),
            case(96_000, SampleKind::F32, 2, false),
            case(16_000, SampleKind::F32, 1, true),
            case(24_000, SampleKind::I16, 1, true),
            case(48_000, SampleKind::I16, 2, false),
            case(44_100, SampleKind::I24, 2, true),
            case(48_000, SampleKind::I32, 6, true),
            case(88_200, SampleKind::F64, 2, true),
        ];

        for Case {
            rate,
            kind,
            channels,
            interleaved,
        } in cases
        {
            let label = format!("{rate} Hz {kind:?} x{channels} interleaved={interleaved}");
            let tone = sine(1_000.0, rate, rate as usize);
            let encode = |samples: &[f32]| -> Vec<u8> {
                match kind {
                    SampleKind::F32 => bytes_of(samples, f32::to_ne_bytes),
                    SampleKind::F64 => bytes_of(samples, |s| f64::from(s).to_ne_bytes()),
                    SampleKind::I16 => {
                        bytes_of(samples, |s| ((s * 32_767.0).round() as i16).to_ne_bytes())
                    }
                    SampleKind::I24 => bytes_of(samples, i24_bytes),
                    SampleKind::I32 => bytes_of(samples, |s| {
                        ((f64::from(s) * 2_147_483_647.0).round() as i32).to_ne_bytes()
                    }),
                }
            };

            let (events, log) = logging_events();
            let context = IoContext::new(events, kind, rate);
            let per_callback = rate as usize / 100;
            for chunk in tone.chunks(per_callback) {
                let time = timestamp(Duration::ZERO);
                if interleaved {
                    let frames: Vec<f32> = chunk.iter().flat_map(|s| vec![*s; channels]).collect();
                    let bytes = encode(&frames);
                    let list = BufferList::over([&bytes[..]], channels as u32);
                    call_io_proc(&context, list.as_ptr(), &time);
                } else {
                    assert_eq!(channels, 2, "planar cases are stereo");
                    let bytes = encode(chunk);
                    let list = BufferList::over([&bytes[..], &bytes[..]], 1);
                    call_io_proc(&context, list.as_ptr(), &time);
                }
            }
            drop(context);

            // What the capture thread does with the buffers.
            let (tx, rx) = frame_channel();
            let start = Instant::now();
            let mut framer = Framer::new(Speaker::Them, start, tx, rate, 1).unwrap();
            let buffers = log.data();
            assert_eq!(buffers.len(), 100, "{label}");
            for (i, (mono, _)) in buffers.iter().enumerate() {
                // Sample-count timing: stamp the buffers 10 ms apart.
                let at = start + Duration::from_millis(10 * i as u64);
                framer.push_mono(mono, at).unwrap();
            }
            framer.finish().unwrap();
            drop(framer);

            let frames: Vec<AudioFrame> = rx.iter().collect();
            let samples: Vec<f32> = frames.iter().flat_map(|f| f.samples.clone()).collect();
            assert_eq!(samples.len(), 16_000, "{label}");
            assert!(frames.iter().all(|f| f.speaker == Speaker::Them), "{label}");
            for (i, frame) in frames.iter().enumerate() {
                assert_eq!(frame.start_ms, 20 * i as u64, "{label}");
            }
            // The tone survives: right pitch, right level, nothing else.
            let steady = &samples[1_600..14_400];
            let frequency = zero_crossing_frequency(steady, 16_000);
            assert!((frequency - 1_000.0).abs() < 1.0, "{label}: {frequency} Hz");
            let fit = fit_sine(steady, 1_000.0, 16_000, 1_600);
            assert!(
                (fit.amplitude - 1.0).abs() < 0.01,
                "{label}: {}",
                fit.amplitude
            );
            assert!(fit.residual_rms < 0.005, "{label}: {}", fit.residual_rms);

            // And it equals the one-shot conversion of the same input.
            if kind == SampleKind::F32 && channels == 1 {
                let direct = to_16k_mono(&tone, 1, rate).unwrap();
                assert_close(&samples, &direct, 1e-4);
            }
        }
    }

    /// Keeps the helpers above honest about `AudioBufferList`'s layout.
    #[test]
    fn the_test_buffer_list_matches_core_audio_layout() {
        assert_eq!(size_of::<BufferList<1>>(), size_of::<AudioBufferList>());
        assert_eq!(align_of::<BufferList<3>>(), align_of::<AudioBufferList>());
        assert_eq!(
            std::mem::offset_of!(BufferList<2>, buffers),
            std::mem::offset_of!(AudioBufferList, mBuffers)
        );
    }
}

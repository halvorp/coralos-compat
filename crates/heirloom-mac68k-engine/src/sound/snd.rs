//! The formats of the Sound Manager as pure functions over bytes: the `'snd '` resource (Inside Macintosh VI, "Sound Manager",
//! format 1 and format 2; Executor `ROMlib_get_snd_cmds` agrees) and the sound headers a `bufferCmd` points at (standard, extended;
//! the compressed header is refused by name). Decoding gives interleaved PCM in the audio seam's formats.
//!
//! ```text
//! 'snd ' format 1: format 1 (word)  numSynths (word)  { synthID (word)  initOption (long) } x numSynths  numCmds (word)  cmds
//! 'snd ' format 2: format 2 (word)  refCount (word)   numCmds (word)  cmds
//! command (8 bytes): cmd (word; bit 15 = param2 is an offset into the resource)  param1 (word)  param2 (long)
//! standard header:  samplePtr 0 (long, 0 = the samples follow)  length 4  sampleRate 8 (Fixed)  loopStart 12  loopEnd 16
//!                   encode 20 (byte: 0 standard, $FF extended, $FE compressed)  baseFrequency 21  samples 22
//! extended header:  as above but `length` is numChannels; numFrames 22  AIFFSampleRate 26 (10 bytes)  markerChunk 36
//!                   instrumentChunks 40  AESRecording 44  sampleSize 48 (word: 8 or 16)  futureUse 50..64  samples 64
//! ```
use crate::Refusal;
use alloc::format;
use alloc::vec::Vec;
use heirloom_ui_abi::AudioFormat;

pub const NULL_CMD: u16 = 0;
pub const QUIET_CMD: u16 = 3;
pub const FLUSH_CMD: u16 = 4;
pub const RE_INIT_CMD: u16 = 5;
pub const WAIT_CMD: u16 = 10;
pub const PAUSE_CMD: u16 = 11;
pub const RESUME_CMD: u16 = 12;
pub const CALL_BACK_CMD: u16 = 13;
pub const SYNC_CMD: u16 = 14;
pub const AVAILABLE_CMD: u16 = 24;
pub const VERSION_CMD: u16 = 25;
pub const VOLUME_CMD: u16 = 46;
pub const GET_VOLUME_CMD: u16 = 47;
pub const SOUND_CMD: u16 = 80;
pub const BUFFER_CMD: u16 = 81;
pub const RATE_CMD: u16 = 82;
pub const GET_RATE_CMD: u16 = 85;

/// One command of a `'snd '` resource or a `SndCommand`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cmd { pub cmd: u16, pub param1: i16, pub param2: u32 }

/// Where in a resource body a header sits (`param2` of a data command with bit 15 set is an offset into the resource).
pub const OFFSET_FLAG: u16 = 0x8000;

use heirloom_common::bytes::{be16, be32};

/// The commands of a `'snd '` resource body, `param2` of an offset command already turned into an offset from the start of the body
/// (the caller adds the body's address). Format 2 turns its first `soundCmd` into `bufferCmd`, as the ROM and Executor do.
pub fn parse_snd(body: &[u8]) -> Result<Vec<(Cmd, bool)>, Refusal> {
    let bad = |why: &str| Refusal(format!("'snd ' resource: {why}"));
    let format = be16(body, 0).ok_or_else(|| bad("shorter than its format word"))?;
    let mut at = match format {
        1 => {
            let n = be16(body, 2).ok_or_else(|| bad("no synthesizer count"))? as usize;
            4 + 6 * n
        }
        2 => 4,
        other => return Err(bad(&format!("format {other} (only formats 1 and 2 exist)"))),
    };
    let n = be16(body, at).ok_or_else(|| bad("no command count"))? as usize;
    at += 2;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let o = at + 8 * i;
        let (c, p1, p2) = (be16(body, o), be16(body, o + 2), be32(body, o + 4));
        let (Some(c), Some(p1), Some(p2)) = (c, p1, p2) else { return Err(bad(&format!("command {i} runs past the end"))) };
        let by_offset = c & OFFSET_FLAG != 0;
        let mut cmd = c & !OFFSET_FLAG;
        if format == 2 && i == 0 && cmd == SOUND_CMD { cmd = BUFFER_CMD; }
        out.push((Cmd { cmd, param1: p1 as i16, param2: p2 }, by_offset));
    }
    Ok(out)
}

/// A decoded sound: PCM in the seam's layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sound { pub rate_hz: u32, pub channels: u8, pub format: AudioFormat, pub pcm: Vec<u8>, /** the loop, in FRAMES from the start of the sound: `loopStart .. loopEnd`, when `loopEnd - loopStart > 1` */ pub loop_frames: Option<(u32, u32)> }

/// Hz from a `Fixed` (16.16) sample rate, rounded down as the hardware's integer divider does.
pub fn hz(fixed: u32) -> u32 { fixed >> 16 }

/// Decode the sound header at the start of `h` (the samples may follow it in `h`, or live at `h`'s `samplePtr`, which the caller
/// resolves and passes as `external`).
pub fn decode_header(h: &[u8], external: impl Fn(u32, u32) -> Option<Vec<u8>>) -> Result<Sound, Refusal> {
    let bad = |why: &str| Refusal(format!("sound header: {why}"));
    let (sample_ptr, len_or_chans, rate) = (be32(h, 0).ok_or_else(|| bad("shorter than 22 bytes"))?, be32(h, 4).ok_or_else(|| bad("shorter than 22 bytes"))?, be32(h, 8).ok_or_else(|| bad("shorter than 22 bytes"))?);
    let (loop_start, loop_end) = (be32(h, 12).ok_or_else(|| bad("no loop points"))?, be32(h, 16).ok_or_else(|| bad("no loop points"))?);
    let encode = *h.get(20).ok_or_else(|| bad("no encode byte"))?;
    let rate_hz = hz(rate);
    if rate_hz == 0 { return Err(bad("sample rate 0")); }
    let loop_frames = (loop_end.saturating_sub(loop_start) > 1).then_some((loop_start, loop_end));
    match encode {
        0 => {
            let samples = if sample_ptr == 0 { h.get(22..22 + len_or_chans as usize).map(<[u8]>::to_vec).ok_or_else(|| bad(&format!("{len_or_chans} samples run past the end of the resource")))? }
                          else { external(sample_ptr, len_or_chans).ok_or_else(|| bad(&format!("samplePtr {sample_ptr:#x} is not readable memory")))? };
            Ok(Sound { rate_hz, channels: 1, format: AudioFormat::U8, pcm: samples, loop_frames })
        }
        0xFF => {
            let (channels, frames, size) = (len_or_chans, be32(h, 22).ok_or_else(|| bad("no numFrames"))?, be16(h, 48).ok_or_else(|| bad("no sampleSize"))?);
            if !(1..=2).contains(&channels) { return Err(bad(&format!("{channels} channels (the seam plays mono and stereo)"))); }
            let bytes = frames as usize * channels as usize * (size as usize / 8);
            let raw = if sample_ptr == 0 { h.get(64..64 + bytes).map(<[u8]>::to_vec).ok_or_else(|| bad("the extended header's samples run past the end of the resource"))? }
                      else { external(sample_ptr, bytes as u32).ok_or_else(|| bad(&format!("samplePtr {sample_ptr:#x} is not readable memory")))? };
            match size {
                8 => Ok(Sound { rate_hz, channels: channels as u8, format: AudioFormat::U8, pcm: raw, loop_frames }),
                16 => {
                    // two's complement, big-endian on the Mac, little-endian in the seam
                    let mut pcm = raw;
                    for s in pcm.chunks_exact_mut(2) { s.swap(0, 1); }
                    Ok(Sound { rate_hz, channels: channels as u8, format: AudioFormat::S16Le, pcm, loop_frames })
                }
                other => Err(bad(&format!("{other}-bit samples (8 and 16 are framed)"))),
            }
        }
        0xFE => Err(bad("a compressed sound header (MACE or IMA): no decompressor is modelled")),
        other => Err(bad(&format!("encode byte {other:#04x} is not standard (0), extended ($FF) or compressed ($FE)"))),
    }
}

/// A short system beep: a sine at `freq` Hz, `ms` long, 8-bit unsigned mono at `rate` Hz, with a linear fade-out (no click). The sine is
/// Bhaskara's parabolic approximation (error under 0.2 %), so the engine needs no maths library for a beep.
pub fn beep(freq: u32, ms: u32, rate: u32) -> Vec<u8> {
    let n = (u64::from(rate) * u64::from(ms) / 1000) as usize;
    (0..n).map(|i| {
        // phase in [0, 1) of the period; each half period is sin(pi t) ~ 16 t (1 - t) / (5 - 4 t (1 - t)), t in [0, 1]
        let ph = ((i as u64 * u64::from(freq)) % u64::from(rate)) as f32 / rate as f32;
        let (t, sign) = if ph < 0.5 { (ph * 2.0, 1.0) } else { ((ph - 0.5) * 2.0, -1.0) };
        let p = t * (1.0 - t);
        let sine = sign * 16.0 * p / (5.0 - 4.0 * p);
        let fade = 1.0 - i as f32 / n.max(1) as f32;
        (128.0 + (sine * 0.5 * fade).clamp(-1.0, 1.0) * 127.0) as u8
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std_header(rate: u32, samples: &[u8]) -> Vec<u8> {
        let mut h = alloc::vec![0, 0, 0, 0];
        h.extend((samples.len() as u32).to_be_bytes());
        h.extend(rate.to_be_bytes());
        h.extend([0u8; 8]);
        h.extend([0, 60]);
        h.extend(samples);
        h
    }

    #[test]
    fn a_format_1_resource_yields_its_commands_with_offsets_flagged() {
        // format 1, one synth (sampledSynth 5, init 0), one command: bufferCmd | $8000, param1 0, param2 = offset 20
        let mut b = alloc::vec![0, 1, 0, 1, 0, 5, 0, 0, 0, 0, 0, 1];
        b.extend([0x80, 81, 0, 0, 0, 0, 0, 20]);
        let c = parse_snd(&b).unwrap();
        assert_eq!(c, [(Cmd { cmd: BUFFER_CMD, param1: 0, param2: 20 }, true)]);
    }

    #[test]
    fn format_2_turns_its_first_soundcmd_into_buffercmd_and_other_formats_are_refused() {
        let mut b = alloc::vec![0, 2, 0, 1, 0, 2];
        b.extend([0x80, 80, 0, 0, 0, 0, 0, 14, 0, 80, 0, 0, 0, 0, 0, 14]);
        let c = parse_snd(&b).unwrap();
        assert_eq!((c[0].0.cmd, c[1].0.cmd), (BUFFER_CMD, SOUND_CMD), "only command 0 is rewritten");
        assert!(parse_snd(&[0, 3, 0, 0]).unwrap_err().0.contains("format 3"));
        let mut cut = alloc::vec![0, 2, 0, 1, 0, 2];
        cut.extend([0x80, 80, 0, 0, 0, 0, 0, 14]);
        assert!(parse_snd(&cut).unwrap_err().0.contains("command 1 runs past the end"), "a command count past the data is refused, not read");
    }

    #[test]
    fn a_standard_header_is_unsigned_8_bit_mono_at_the_rate_in_whole_hz() {
        let s = decode_header(&std_header(0x56EE_8BA3, &[0x80, 0x90, 0x70]), |_, _| None).unwrap();
        assert_eq!((s.rate_hz, s.channels, s.format, s.pcm.as_slice(), s.loop_frames), (22254, 1, AudioFormat::U8, &[0x80u8, 0x90, 0x70][..], None));
        // samplePtr: the samples live elsewhere
        let mut h = std_header(0x2B77_0000, &[]);
        h[3] = 1;
        h[7] = 2;
        let s = decode_header(&h, |p, n| (p == 1 && n == 2).then(|| alloc::vec![9, 8])).unwrap();
        assert_eq!((s.rate_hz, s.pcm), (11127, alloc::vec![9, 8]));
        assert!(decode_header(&h, |_, _| None).unwrap_err().0.contains("not readable"));
    }

    #[test]
    fn an_extended_header_decodes_16_bit_stereo_big_endian_to_little_endian() {
        let mut h = alloc::vec![0u8; 64];
        h[7] = 2; // numChannels
        h[8..12].copy_from_slice(&0x5622_0000u32.to_be_bytes()); // 22050
        h[20] = 0xFF;
        h[25] = 2; // numFrames
        h[49] = 16; // sampleSize
        h.extend([0x01, 0x02, 0xFF, 0xFE, 0x00, 0x10, 0x7F, 0xFF]);
        let s = decode_header(&h, |_, _| None).unwrap();
        assert_eq!((s.rate_hz, s.channels, s.format), (22050, 2, AudioFormat::S16Le));
        assert_eq!(s.pcm, [0x02, 0x01, 0xFE, 0xFF, 0x10, 0x00, 0xFF, 0x7F]);
        // negative control: 8-bit extended is not byte-swapped
        h[49] = 8;
        let s = decode_header(&h, |_, _| None).unwrap();
        assert_eq!((s.format, s.pcm.len()), (AudioFormat::U8, 4));
    }

    #[test]
    fn compressed_zero_rate_and_loops_are_named() {
        let mut h = std_header(0x56EE_8BA3, &[1, 2, 3]);
        h[20] = 0xFE;
        assert!(decode_header(&h, |_, _| None).unwrap_err().0.contains("compressed"));
        assert!(decode_header(&std_header(0, &[1]), |_, _| None).unwrap_err().0.contains("rate 0"));
        let mut l = std_header(0x56EE_8BA3, &[1, 2, 3, 4]);
        l[16..20].copy_from_slice(&3u32.to_be_bytes());
        assert_eq!(decode_header(&l, |_, _| None).unwrap().loop_frames, Some((0, 3)), "loopEnd - loopStart > 1 is a looped sound; the points are kept");
    }

    #[test]
    fn the_beep_fades_to_silence_and_has_the_length_asked() {
        let b = beep(800, 100, 22254);
        assert_eq!(b.len(), 2225);
        assert!(b.iter().any(|&x| x > 160) && b.iter().any(|&x| x < 96), "it swings both ways");
        assert!(b[b.len() - 20..].iter().all(|&x| (120..136).contains(&x)), "the tail is faded");
    }
}

//! Tiny PCM16LE → WAV-container wrapper used to ship audio batches to
//! the backend Whisper endpoint.
//!
//! `voice::cloud_transcribe` takes whatever the desktop UI captured
//! (typically `audio/webm`) and forwards bytes to the backend. Our
//! call buffers are raw PCM16LE @ 16 kHz mono — Whisper accepts WAV
//! natively, so we wrap the bytes in a minimal RIFF/WAVE header and
//! mark the upload as `audio/wav`. No other transcoding needed.

const WAV_HEADER_LEN: usize = 44;

/// Produce a complete WAV file (header + interleaved PCM16LE samples).
/// Caller passes the raw `i16` slice and the sample rate; mono is
/// hard-coded because that's what the meet-agent loop uses end-to-end.
pub fn pack_pcm16le_mono_wav(samples: &[i16], sample_rate_hz: u32) -> Vec<u8> {
    let data_bytes = samples.len() * 2;
    let mut out = Vec::with_capacity(WAV_HEADER_LEN + data_bytes);

    // RIFF chunk descriptor
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    // fmt sub-chunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM header size
    out.extend_from_slice(&1u16.to_le_bytes()); // audio format = PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // num channels = 1
    out.extend_from_slice(&sample_rate_hz.to_le_bytes());
    out.extend_from_slice(&(sample_rate_hz * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    // data sub-chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_bytes_match_riff_wave_layout() {
        let bytes = pack_pcm16le_mono_wav(&[0; 8000], 16_000);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(&bytes[36..40], b"data");
        // RIFF size = 36 + data_bytes (8000 samples * 2 bytes = 16000).
        let riff_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        assert_eq!(riff_size, 36 + 16_000);
        // Sample rate field at offset 24.
        let rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
        assert_eq!(rate, 16_000);
    }

    #[test]
    fn empty_input_still_produces_valid_header() {
        let bytes = pack_pcm16le_mono_wav(&[], 16_000);
        assert_eq!(bytes.len(), WAV_HEADER_LEN);
        assert_eq!(&bytes[0..4], b"RIFF");
    }

    #[test]
    fn samples_are_appended_little_endian() {
        let bytes = pack_pcm16le_mono_wav(&[0x1234, -1], 16_000);
        // First sample 0x1234 → LE bytes 0x34, 0x12 starting at offset 44.
        assert_eq!(bytes[44], 0x34);
        assert_eq!(bytes[45], 0x12);
        // -1 in i16 LE → 0xFF, 0xFF.
        assert_eq!(bytes[46], 0xFF);
        assert_eq!(bytes[47], 0xFF);
    }
}

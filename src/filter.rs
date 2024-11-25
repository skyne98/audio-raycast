use fundsp::hacker32::*;

pub fn process_audio_bands(samples: &mut [f32], bands: [f32; 5], chunk_size: usize) {
    let mut filters = [
        lowpass_hz(200.0, 1.0),
        lowpass_hz(600.0, 1.0),
        lowpass_hz(1200.0, 1.0),
        lowpass_hz(3000.0, 1.0),
        lowpass_hz(20000.0, 1.0),
    ];

    // Pre-allocate a single buffer and reuse it
    let mut processed_chunk = vec![0.0; chunk_size];

    for chunk in samples.chunks_mut(chunk_size) {
        // Clear the buffer instead of reallocating
        processed_chunk[..chunk.len()].fill(0.0);

        // Process all samples through each filter
        for (filter, &adjustment) in filters.iter_mut().zip(bands.iter()) {
            for (j, &sample) in chunk.iter().enumerate() {
                processed_chunk[j] += adjustment * filter.filter_mono(sample);
            }
        }

        // Copy back the results
        chunk.copy_from_slice(&processed_chunk[..chunk.len()]);
    }
}

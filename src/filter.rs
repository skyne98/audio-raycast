use fundsp::hacker32::*;

pub enum FilterType {
    Lowpass(An<FixedSvf<f32, LowpassMode<f32>>>),
    Bandpass(An<FixedSvf<f32, BandpassMode<f32>>>),
    Highpass(An<FixedSvf<f32, HighpassMode<f32>>>),
}

pub struct AudioBandProcessor {
    filters: [FilterType; 5],
    bands: [f32; 5],
}

impl AudioBandProcessor {
    pub fn new() -> Self {
        Self {
            filters: [
                FilterType::Lowpass(lowpass_hz(200.0, 1.0)), // Low band (0 - 200 Hz)
                FilterType::Bandpass(bandpass_hz(400.0, 1.0)), // Low-mid band (200 - 600 Hz)
                FilterType::Bandpass(bandpass_hz(900.0, 1.0)), // Mid band (600 - 1200 Hz)
                FilterType::Bandpass(bandpass_hz(2100.0, 1.0)), // Upper-mid band (1200 - 3000 Hz)
                FilterType::Highpass(highpass_hz(3000.0, 1.0)), // High band (3000 Hz +)
            ],
            bands: [1.0; 5], // Default gain values for each band
        }
    }

    pub fn update_bands(&mut self, new_bands: [f32; 5]) {
        self.bands = new_bands;
    }

    pub fn process_buffer(&mut self, input: &[f32], output: &mut [f32]) {
        // Clear the output buffer
        output.fill(0.0);

        // Process each filter
        for (i, filter) in self.filters.iter_mut().enumerate() {
            // Create a temporary buffer for this filter's output
            let mut temp_buffer = vec![0.0; input.len()];

            // Process the entire buffer through this filter
            match filter {
                FilterType::Lowpass(node) => {
                    for (j, &sample) in input.iter().enumerate() {
                        temp_buffer[j] = self.bands[i] * node.filter_mono(sample);
                    }
                }
                FilterType::Bandpass(node) => {
                    for (j, &sample) in input.iter().enumerate() {
                        temp_buffer[j] = self.bands[i] * node.filter_mono(sample);
                    }
                }
                FilterType::Highpass(node) => {
                    for (j, &sample) in input.iter().enumerate() {
                        temp_buffer[j] = self.bands[i] * node.filter_mono(sample);
                    }
                }
            }

            // Add this filter's output to the main output buffer
            for (out, &temp) in output.iter_mut().zip(temp_buffer.iter()) {
                *out += temp;
            }
        }
    }
}

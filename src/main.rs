use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use filter::AudioBandProcessor;
use tracing::{error, info};

pub mod filter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("no output device available");
    info!("Default output device: {:#?}", device.name());

    let config = device.default_output_config()?;
    info!("Default output config: {:#?}", config);

    let sample_rate = config.sample_rate().0 as f32;
    info!("Sample rate: {}", sample_rate);

    let file = tokio::fs::read("assets/sample-0.wav").await?;
    let mut reader = hound::WavReader::new(&file[..])?;
    let spec = reader.spec();
    info!("WAV file spec: {:#?}", spec);
    let wav_sample_rate = spec.sample_rate as f32;
    let ratio = sample_rate / wav_sample_rate;
    let sample_type = spec.sample_format;
    let samples = match sample_type {
        hound::SampleFormat::Int => {
            let samples: Vec<i32> = reader.samples::<i32>().map(|s| s.unwrap()).collect();
            samples
                .iter()
                // Normalize integer samples to [-1.0, 1.0]
                .map(|s| *s as f32 / i32::MAX as f32)
                .collect::<Vec<f32>>()
        }
        hound::SampleFormat::Float => {
            // Float samples are already normalized
            reader.samples::<f32>().map(|s| s.unwrap()).collect()
        }
    };
    info!("Read {} samples from the WAV file", samples.len());

    let target_peak = 0.7; // Target peak amplitude (70% of max)
    let max_amplitude = samples.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    let auto_gain = if max_amplitude > 0.0 {
        target_peak / max_amplitude
    } else {
        1.0
    };

    // Apply gain with soft limiting
    let samples: Vec<f32> = samples
        .iter()
        .map(|&s| {
            let amplified = s * auto_gain;
            // Soft limiting to prevent harsh clipping
            if amplified.abs() > 0.8 {
                0.8 * amplified.signum() + 0.2 * (amplified - 0.8 * amplified.signum())
            } else {
                amplified
            }
        })
        .collect();

    // Make new samples with the correct sample rate via linear interpolation
    let mut new_samples = Vec::new();
    let output_len = (samples.len() as f32 * ratio) as usize;
    for i in 0..output_len {
        let pos = i as f32 / ratio;
        let pos_floor = pos.floor() as usize;
        let pos_ceil = (pos_floor + 1).min(samples.len() - 1);
        let frac = pos - pos_floor as f32;

        let sample = samples[pos_floor] * (1.0 - frac) + samples[pos_ceil] * frac;
        new_samples.push(sample);
    }
    let mut samples = new_samples;

    // Apply a muffle effect to the audio
    let mut filter = AudioBandProcessor::new();
    filter.update_bands(/* muffle behind a door */ [1.0, 0.25, 0.0, 0.0, 0.0]);

    let channels = config.channels() as usize;
    let mut current_sample = 0;
    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                if let Some(sample) = samples.get(current_sample) {
                    let sample = filter.process_sample(*sample);
                    for channel in frame.iter_mut() {
                        *channel = sample;
                    }
                    current_sample += 1;
                } else {
                    for channel in frame.iter_mut() {
                        *channel = 0.0;
                    }
                }
            }
        },
        move |err| {
            error!("an error occurred on the output stream: {:#?}", err);
        },
        None,
    );

    info!("Starting audio playback...");
    let stream = stream?;
    stream.play()?;
    info!("Stream is playing. Press Ctrl+C to stop.");

    // Keep the stream alive
    tokio::signal::ctrl_c().await?;
    info!("Stopping audio playback...");

    Ok(())
}

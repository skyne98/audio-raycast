use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound;
use std::sync::{Arc, Mutex};

fn main() -> Result<(), anyhow::Error> {
    // Read WAV file
    let mut reader = hound::WavReader::open("./assets/sample-0.wav")?;
    let spec = reader.spec();
    let wav_sample_rate = spec.sample_rate as f32;

    // Convert samples to f32 based on format
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => match spec.bits_per_sample {
            16 => reader
                .samples::<i16>()
                .map(|s| s.unwrap() as f32 / i16::MAX as f32)
                .collect(),
            24 | 32 => reader
                .samples::<i32>()
                .map(|s| s.unwrap() as f32 / i32::MAX as f32)
                .collect(),
            8 => reader
                .samples::<i8>()
                .map(|s| s.unwrap() as f32 / i8::MAX as f32)
                .collect(),
            _ => return Err(anyhow::anyhow!("Unsupported bit depth")),
        },
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
    };

    let samples = Arc::new(Mutex::new(samples));

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("no output device available");
    let config = device.default_output_config()?;
    let channels = config.channels();
    let output_sample_rate = config.sample_rate().0 as f32;

    // Calculate playback speed ratio
    let speed_ratio = wav_sample_rate / output_sample_rate;
    let position = Arc::new(Mutex::new(0.0f32));

    let samples_clone = samples.clone();
    let position_clone = position.clone();

    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let samples = samples_clone.lock().unwrap();
            let mut pos = position_clone.lock().unwrap();

            for frame in data.chunks_mut(channels as usize) {
                let sample_pos = *pos as usize;
                let value = if sample_pos < samples.len() {
                    // Linear interpolation between samples
                    let fract = *pos - sample_pos as f32;
                    let current = samples[sample_pos];
                    let next = if sample_pos + 1 < samples.len() {
                        samples[sample_pos + 1]
                    } else {
                        0.0
                    };
                    current * (1.0 - fract) + next * fract
                } else {
                    0.0
                };

                for sample in frame.iter_mut() {
                    *sample = value;
                }
                *pos += speed_ratio;
            }
        },
        |err| eprintln!("Error: {}", err),
        None,
    )?;

    stream.play()?;

    while *position.lock().unwrap() < samples.lock().unwrap().len() as f32 {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    Ok(())
}

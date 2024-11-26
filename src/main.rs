use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::vec::Vec;
use tracing::{error, info};

// Inside your `run()` function
async fn run() -> Result<()> {
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

    let sample_type = config.sample_format();
    info!("Sample type: {:#?}", sample_type);

    // Read sample from a file
    let samples = hound::WavReader::open("assets/sample-0.wav")?;
    let wav_sample_type = samples.spec().sample_format;
    let wav_sample_rate = samples.spec().sample_rate as f32;
    let samples: Vec<f32> = match wav_sample_type {
        hound::SampleFormat::Float => samples.into_samples::<f32>().map(|s| s.unwrap()).collect(),
        hound::SampleFormat::Int => samples
            .into_samples::<i16>()
            .map(|s| s.unwrap() as f32 / i16::MAX as f32)
            .collect(),
    };
    info!("Read {} samples from the file", samples.len());

    // Resample the samples to the output sample rate using linear interpolation
    let resample_ratio = sample_rate / wav_sample_rate;
    let resampled_length = ((samples.len() as f32) * resample_ratio) as usize;
    let mut resampled_samples = Vec::with_capacity(resampled_length);

    for i in 0..resampled_length {
        let src_index = i as f32 / resample_ratio;
        let index_floor = src_index.floor() as usize;
        let index_ceil = (index_floor + 1).min(samples.len() - 1);
        let frac = src_index - index_floor as f32;
        let sample = samples[index_floor] * (1.0 - frac) + samples[index_ceil] * frac;
        resampled_samples.push(sample);
    }
    let samples = resampled_samples;

    // Initialize HRTF processor
    let hrtf_sphere = hrtf::HrirSphere::from_file("assets/hrir.bin", sample_rate as u32)
        .map_err(|e| anyhow::anyhow!("Failed to load HRTF data: {:#?}", e))?;
    let processor = hrtf::HrtfProcessor::new(hrtf_sphere, 8, 128);

    // Create channels for communication
    let (input_tx, input_rx): (Sender<Vec<f32>>, Receiver<Vec<f32>>) = mpsc::channel();
    let (output_tx, output_rx): (Sender<Vec<f32>>, Receiver<Vec<f32>>) = mpsc::channel();

    // Spawn a thread to process audio chunks
    thread::spawn(move || {
        process_audio_chunks(processor, input_rx, output_tx);
    });

    // Spawn a thread to feed samples into the input channel
    let samples_clone = samples.clone();
    thread::spawn(move || {
        let chunk_size = 1024;
        for chunk in samples_clone.chunks(chunk_size) {
            // Pad the last chunk with zeros
            let chunk = if chunk.len() < chunk_size {
                let mut padded_chunk = vec![0.0; chunk_size];
                padded_chunk[..chunk.len()].copy_from_slice(chunk);
                padded_chunk
            } else {
                chunk.to_vec()
            };
            if input_tx.send(chunk.to_vec()).is_err() {
                break;
            }
        }
    });

    // Build the output stream
    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut idx = 0;
            while idx < data.len() {
                match output_rx.recv() {
                    Ok(buffer) => {
                        for &sample in &buffer {
                            if idx < data.len() {
                                data[idx] = sample;
                                idx += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    Err(_) => {
                        // No more data, fill with silence
                        for sample in &mut data[idx..] {
                            *sample = 0.0;
                        }
                        break;
                    }
                }
            }
        },
        move |err| {
            error!("An error occurred on the output stream: {:#?}", err);
        },
        None,
    )?;

    stream.play()?;
    info!("Stream is playing. Press Ctrl+C to stop.");

    // Wait for Ctrl+C
    tokio::signal::ctrl_c().await?;

    Ok(())
}

// Function to process audio chunks
fn process_audio_chunks(
    mut processor: hrtf::HrtfProcessor,
    input_rx: Receiver<Vec<f32>>,
    output_tx: Sender<Vec<f32>>,
) {
    let mut prev_left_samples = vec![];
    let mut prev_right_samples = vec![];
    let mut previous_sample_vector = hrtf::Vec3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let start_time = std::time::Instant::now();

    while let Ok(buffer) = input_rx.recv() {
        let mut output = vec![(0.0f32, 0.0f32); buffer.len()];

        // Update sample vector based on time or your desired parameters
        let time_since = start_time.elapsed().as_secs_f32();
        let angle = time_since * std::f32::consts::TAU;
        let new_sample_vector = hrtf::Vec3 {
            x: angle.cos(),
            y: angle.sin(),
            z: 0.0,
        };

        let context = hrtf::HrtfContext {
            source: &buffer,
            output: &mut output,
            new_sample_vector,
            prev_sample_vector: previous_sample_vector,
            prev_left_samples: &mut prev_left_samples,
            prev_right_samples: &mut prev_right_samples,
            new_distance_gain: 1.0,
            prev_distance_gain: 1.0,
        };

        processor.process_samples(context);
        previous_sample_vector = new_sample_vector;

        // Flatten stereo samples into a single Vec<f32>
        let stereo_buffer: Vec<f32> = output
            .iter()
            .flat_map(|&(left, right)| vec![left, right])
            .collect();

        if output_tx.send(stereo_buffer).is_err() {
            break;
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    run().await
}

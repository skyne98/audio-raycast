use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use filter::AudioBandProcessor;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::vec::Vec;
use tracing::{error, info};

const INTERPOLATION_STEPS: usize = 8;
const BLOCK_LEN: usize = 128;
const CHUNK: usize = INTERPOLATION_STEPS * BLOCK_LEN;

mod filter;

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

    // Read sample from a file
    let samples = hound::WavReader::open("assets/sample-0.wav")?;
    let wav_sample_type = samples.spec().sample_format;
    let wav_sample_rate = samples.spec().sample_rate as f32;
    let spec = samples.spec();
    let samples: Vec<f32> = match wav_sample_type {
        hound::SampleFormat::Float => samples
            .into_samples::<f32>()
            .filter_map(Result::ok)
            .collect(),
        hound::SampleFormat::Int => samples
            .into_samples::<i16>()
            .filter_map(Result::ok)
            .map(|s| s as f32 / i16::MAX as f32)
            .collect(),
    };
    info!("Audio sample spec: {:#?}", spec);

    // Resample to the output sample rate linear interpolation
    let resample_ratio = sample_rate / wav_sample_rate;
    let resampled_length = ((samples.len() as f32) * resample_ratio) as usize;
    let mut resampled_samples = Vec::with_capacity(resampled_length);
    for i in 0..resampled_length {
        let src_index = i as f32 / resample_ratio;
        let index_floor = src_index.floor() as usize;
        let index_ceil = (index_floor + 1).min(samples.len() - 1);
        let weight = src_index - index_floor as f32;
        let sample = samples[index_floor] * (1.0 - weight) + samples[index_ceil] * weight;
        resampled_samples.push(sample);
    }

    // Initialize HRTF processor
    let hrtf_sphere = hrtf::HrirSphere::from_file("assets/hrir-1.bin", sample_rate as u32)
        .map_err(|e| anyhow::anyhow!("Failed to load HRTF data: {:#?}", e))?;
    let processor = hrtf::HrtfProcessor::new(hrtf_sphere, INTERPOLATION_STEPS, BLOCK_LEN);

    // Create channels for communication
    let (input_tx, input_rx): (Sender<Vec<f32>>, Receiver<Vec<f32>>) = mpsc::channel();
    let (output_tx, output_rx): (Sender<Vec<f32>>, Receiver<Vec<f32>>) = mpsc::channel();

    // Spawn a thread to process audio chunks
    thread::spawn(move || {
        process_audio_chunks(processor, input_rx, output_tx, sample_rate);
    });

    // Spawn a thread to feed samples into the input channel
    let samples_clone = resampled_samples.clone();
    thread::spawn(move || {
        let chunk_size = CHUNK;
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
    let mut leftover: Vec<f32> = Vec::new();
    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut idx = 0;
            // Use leftover data first
            if !leftover.is_empty() {
                let len = leftover.len().min(data.len());
                data[..len].copy_from_slice(&leftover[..len]);
                idx += len;
                leftover.drain(..len);
            }
            // Keep fetching buffers until we fill 'data' or no more data is available
            while idx < data.len() {
                match output_rx.try_recv() {
                    Ok(buffer) => {
                        let len = buffer.len().min(data.len() - idx);
                        data[idx..idx + len].copy_from_slice(&buffer[..len]);
                        idx += len;
                        // Store any extra data for the next callback
                        if len < buffer.len() {
                            leftover.extend_from_slice(&buffer[len..]);
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        // No data available, fill the rest with silence to prevent underflow
                        for sample in &mut data[idx..] {
                            *sample = 0.0;
                        }
                        break;
                    }
                    Err(_) => {
                        // Channel disconnected, fill the rest with silence
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
    sample_rate: f32,
) {
    let mut prev_left_samples = vec![];
    let mut prev_right_samples = vec![];
    let mut previous_sample_vector = hrtf::Vec3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let mut prev_distance_gain = 1.0;

    let mut filter = AudioBandProcessor::new();
    filter.update_bands([0.85, 0.25, 0.01, 0.005, 0.001]);

    // Track total samples processed
    let mut total_samples = 0;
    const ROTATIONS_PER_SECOND: f32 = 0.5;
    while let Ok(buffer) = input_rx.recv() {
        let mut filter_output = vec![0.0; buffer.len()];
        filter.process_buffer(&buffer, &mut filter_output);

        let mut output = vec![(0.0f32, 0.0f32); buffer.len()];

        // Calculate angle based on sample position
        let time_in_seconds = total_samples as f32 / sample_rate;
        let angle = time_in_seconds * ROTATIONS_PER_SECOND * std::f32::consts::TAU;

        let new_sample_vector = hrtf::Vec3 {
            x: angle.cos(),
            y: angle.sin(),
            z: 0.0,
        };
        let distance = f32::sqrt(new_sample_vector.x.powi(2) + new_sample_vector.y.powi(2));

        let context = hrtf::HrtfContext {
            source: &filter_output,
            output: &mut output,
            new_sample_vector,
            prev_sample_vector: previous_sample_vector,
            prev_left_samples: &mut prev_left_samples,
            prev_right_samples: &mut prev_right_samples,
            new_distance_gain: distance,
            prev_distance_gain: prev_distance_gain,
        };

        processor.process_samples(context);
        previous_sample_vector = new_sample_vector;
        prev_distance_gain = distance;

        let stereo_buffer: Vec<f32> = output
            .iter()
            .flat_map(|&(left, right)| vec![left, right])
            .collect();

        // Update total samples processed
        total_samples += buffer.len();

        if output_tx.send(stereo_buffer).is_err() {
            break;
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    run().await
}

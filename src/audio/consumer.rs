use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc::Receiver;

pub type SampleBuffer = Vec<f32>;

pub struct ConsumerInfo {
    pub sample_rate: u32,
    pub channels: u16,
}

pub fn start_consumer(sample_receiver: Receiver<SampleBuffer>) -> Result<ConsumerInfo> {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device");
    let config = device.default_output_config()?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels();

    let mut play_buffer = Vec::new();
    let mut last_sample = 0.0;
    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
            // Receive new samples from the producer
            while let Ok(samples) = sample_receiver.try_recv() {
                play_buffer.extend(samples);
            }

            // Fill the output buffer with samples
            for sample in data.iter_mut() {
                *sample = if !play_buffer.is_empty() {
                    last_sample = play_buffer.remove(0);
                    last_sample
                } else {
                    last_sample
                };
            }
        },
        |err| eprintln!("Audio error: {}", err),
        None,
    )?;
    stream.play()?;

    Ok(ConsumerInfo {
        sample_rate,
        channels,
    })
}

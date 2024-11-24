use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tracing::{error, info};

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

    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            // Iterate through each audio sample in the buffer, allowing modification
            for sample in data.iter_mut() {
                // Maintain phase angle across iterations using static variable
                // Must be unsafe since it's static mut
                static mut PHASE: f32 = 0.0;

                unsafe {
                    // Set frequency to A4 note (440Hz)
                    let frequency = 440.0;

                    // Calculate sample value using sine wave:
                    // - sin(PHASE) creates the waveform
                    // - 0.8 multiplier controls volume/amplitude
                    *sample = PHASE.sin() * 0.8;

                    // Advance phase for next sample:
                    // - 2π * frequency gives angular velocity
                    // - Divide by sample_rate converts to per-sample phase increment
                    PHASE += 2.0 * std::f32::consts::PI * frequency / sample_rate;

                    // Keep PHASE between 0 and 2π to prevent floating point precision loss
                    // This is called phase wrapping
                    if PHASE >= 2.0 * std::f32::consts::PI {
                        PHASE -= 2.0 * std::f32::consts::PI;
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

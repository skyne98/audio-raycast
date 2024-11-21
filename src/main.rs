use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::{cursor, terminal, ExecutableCommand};
use num_complex::Complex;
use rustfft::{num_complex::Complex32, FftPlanner};
use std::io::stdout;
use std::sync::{Arc, Mutex};
use textplots::{Chart, Plot, Shape};

struct CircularBuffer {
    data: Vec<f32>,
    position: usize,
    size: usize,
}

impl CircularBuffer {
    fn new(size: usize) -> Self {
        CircularBuffer {
            data: vec![0.0; size],
            position: 0,
            size,
        }
    }

    fn push(&mut self, value: f32) {
        self.data[self.position] = value;
        self.position = (self.position + 1) % self.size;
    }

    fn get_fft_data(&self) -> Vec<(f32, f32)> {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(self.size);

        // Prepare FFT input
        let mut buffer: Vec<Complex32> =
            self.data.iter().map(|&x| Complex32::new(x, 0.0)).collect();

        // Perform FFT
        fft.process(&mut buffer);

        // Convert to magnitude and normalize
        let mut result = Vec::with_capacity(self.size / 2);
        let max_freq = self.size as f32 / 2.0;

        for (i, complex) in buffer.iter().take(self.size / 2).enumerate() {
            let magnitude = (complex.norm() / (self.size as f32).sqrt()).log10() * 20.0;
            result.push((i as f32 / max_freq * 100.0, magnitude));
        }

        result
    }

    fn get_frequency_bands(&self) -> Vec<(f32, f32, String)> {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(self.size);
        let sample_rate = 44100.0; // Standard audio sample rate

        // Prepare FFT input
        let mut buffer: Vec<Complex32> = self
            .data
            .iter()
            .map(|&x| Complex32::new(x * 0.5, 0.0)) // Apply Hanning window
            .collect();

        fft.process(&mut buffer);

        // Define frequency bands (Hz)
        let bands = vec![
            (20.0, 60.0, "Deep Bass", "\x1b[34m"),     // Blue
            (60.0, 250.0, "Bass", "\x1b[36m"),         // Cyan
            (250.0, 500.0, "Low Mid", "\x1b[32m"),     // Green
            (500.0, 2000.0, "Mid", "\x1b[33m"),        // Yellow
            (2000.0, 4000.0, "Upper Mid", "\x1b[31m"), // Red
            (4000.0, 20000.0, "Treble", "\x1b[35m"),   // Magenta
        ];

        let mut result = Vec::new();

        for (low_freq, high_freq, name, color) in bands {
            let low_bin = ((low_freq * self.size as f32) / sample_rate) as usize;
            let high_bin = ((high_freq * self.size as f32) / sample_rate) as usize;

            let magnitude = buffer[low_bin..high_bin]
                .iter()
                .map(|c| c.norm())
                .sum::<f32>()
                / (high_bin - low_bin) as f32;

            let db = 20.0 * (magnitude / (self.size as f32).sqrt()).log10();
            result.push((
                low_freq,
                db.max(-60.0),
                format!("{}{}{}", color, name, "\x1b[0m"),
            ));
        }

        result
    }

    fn get_frequency_bands_smooth(
        &self,
        prev_values: &mut Vec<f32>,
        peaks: &mut Vec<f32>,
    ) -> Vec<(f32, f32, String)> {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(self.size);
        let sample_rate = 44100.0;

        let mut buffer: Vec<Complex32> = self
            .data
            .iter()
            .map(|&x| Complex32::new(x * 0.5, 0.0))
            .collect();

        fft.process(&mut buffer);

        // ISO standard frequencies for 31-band equalizer
        let bands = vec![
            (20.0, 25.0, "20"),
            (25.0, 31.5, "25"),
            (31.5, 40.0, "31"),
            (40.0, 50.0, "40"),
            (50.0, 63.0, "50"),
            (63.0, 80.0, "63"),
            (80.0, 100.0, "80"),
            (100.0, 125.0, "100"),
            (125.0, 160.0, "125"),
            (160.0, 200.0, "160"),
            (200.0, 250.0, "200"),
            (250.0, 315.0, "250"),
            (315.0, 400.0, "315"),
            (400.0, 500.0, "400"),
            (500.0, 630.0, "500"),
            (630.0, 800.0, "630"),
            (800.0, 1000.0, "800"),
            (1000.0, 1250.0, "1k"),
            (1250.0, 1600.0, "1.2k"),
            (1600.0, 2000.0, "1.6k"),
            (2000.0, 2500.0, "2k"),
            (2500.0, 3150.0, "2.5k"),
            (3150.0, 4000.0, "3.1k"),
            (4000.0, 5000.0, "4k"),
            (5000.0, 6300.0, "5k"),
            (6300.0, 8000.0, "6.3k"),
            (8000.0, 10000.0, "8k"),
            (10000.0, 12500.0, "10k"),
            (12500.0, 16000.0, "12k"),
            (16000.0, 20000.0, "16k"),
        ];

        let mut result = Vec::new();
        let smoothing_factor = 0.7;
        let peak_decay = 0.98;

        if prev_values.is_empty() {
            *prev_values = vec![0.0; bands.len()];
            *peaks = vec![0.0; bands.len()];
        }

        for (i, (low_freq, high_freq, name)) in bands.into_iter().enumerate() {
            let low_bin = ((low_freq * self.size as f32) / sample_rate) as usize;
            let high_bin = ((high_freq * self.size as f32) / sample_rate) as usize;

            let magnitude = buffer[low_bin..high_bin]
                .iter()
                .map(|c| c.norm())
                .sum::<f32>()
                / (high_bin - low_bin) as f32;

            let db = 20.0 * (magnitude / (self.size as f32).sqrt()).log10();
            let db = db.max(-60.0);

            prev_values[i] = prev_values[i] * smoothing_factor + db * (1.0 - smoothing_factor);
            peaks[i] = if prev_values[i] > peaks[i] {
                prev_values[i]
            } else {
                peaks[i] * peak_decay
            };

            result.push((low_freq, prev_values[i], name.to_string()));
        }

        result
    }
}

fn main() -> anyhow::Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("failed to find output device");
    println!("Output device: {}", device.name()?);

    let config = device.default_output_config().unwrap();
    println!("Default output config: {:?}", config);

    // Increase buffer size for better frequency resolution
    let buffer = Arc::new(Mutex::new(CircularBuffer::new(1024)));

    let stream = run::<f32>(&device, &config.clone().into(), buffer.clone())?;
    stream.play()?;

    stdout().execute(terminal::Clear(terminal::ClearType::All))?;

    let buffer = Arc::new(Mutex::new(CircularBuffer::new(1024)));
    let stream = run::<f32>(&device, &config.into(), buffer.clone())?;
    stream.play()?;

    // Clear screen once at startup
    println!("\x1B[2J\x1B[1;1H");

    let mut prev_values = Vec::new();
    let mut peaks = Vec::new();
    let height = 20;

    // Clear screen once at startup
    println!("\x1B[2J\x1B[1;1H");

    loop {
        stdout().execute(cursor::MoveTo(0, 0))?;

        let bands = buffer
            .lock()
            .unwrap()
            .get_frequency_bands_smooth(&mut prev_values, &mut peaks);

        // Draw vertical bars
        for y in (0..height).rev() {
            let mut line = String::new();
            for (i, (freq, db, _)) in bands.iter().enumerate() {
                let normalized = ((db + 60.0) / 60.0).max(0.0).min(1.0);
                let threshold = y as f32 / height as f32;

                let char = if normalized > threshold {
                    // Color gradient from blue (low freq) to red (high freq)
                    let hue = (i as f32 / bands.len() as f32) * 360.0;
                    let color = match (hue / 60.0) as i32 {
                        0 => "\x1b[38;5;21m",  // Deep blue
                        1 => "\x1b[38;5;27m",  // Blue
                        2 => "\x1b[38;5;33m",  // Cyan
                        3 => "\x1b[38;5;40m",  // Green
                        4 => "\x1b[38;5;220m", // Yellow
                        5 => "\x1b[38;5;196m", // Red
                        _ => "\x1b[38;5;201m", // Magenta
                    };
                    format!("{}███\x1b[0m", color)
                } else {
                    "···".to_string()
                };
                line.push_str(&char);
            }
            println!("{}\x1B[K", line);
        }

        // Draw frequency labels (every 4th band to avoid crowding)
        let labels: String = bands
            .iter()
            .enumerate()
            .map(|(i, (_, _, name))| {
                if i % 4 == 0 {
                    format!("{:<3}", name)
                } else {
                    "···".to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("");
        println!("{}\x1B[K", labels);

        std::thread::sleep(std::time::Duration::from_millis(33));
    }
}

fn run<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<Mutex<CircularBuffer>>,
) -> Result<cpal::Stream, anyhow::Error>
where
    T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
{
    let sample_rate = config.sample_rate.0 as f32;
    let channels = config.channels as usize;

    let mut sample_clock = 0f32;
    let mut next_value = move || {
        sample_clock = (sample_clock + 1.0) % sample_rate;
        (sample_clock * 440.0 * 2.0 * std::f32::consts::PI / sample_rate).sin()
    };

    let buffer_clone = buffer.clone();
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_data(data, channels, &mut next_value, &buffer_clone)
        },
        |err| eprintln!("an error occurred on stream: {}", err),
        None,
    )?;

    Ok(stream)
}

fn write_data<T>(
    output: &mut [T],
    channels: usize,
    next_sample: &mut dyn FnMut() -> f32,
    buffer: &Arc<Mutex<CircularBuffer>>,
) where
    T: cpal::Sample + cpal::FromSample<f32>,
{
    for frame in output.chunks_mut(channels) {
        let value = next_sample();
        // Store the sample in our visualization buffer
        buffer.lock().unwrap().push(value);
        for sample in frame.iter_mut() {
            *sample = cpal::Sample::from_sample(value);
        }
    }
}

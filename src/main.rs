use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use filter::AudioBandProcessor;
use macroquad::models::draw_cube;
use macroquad::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use tokio::runtime::Runtime;
use tokio::sync::watch;

mod filter;

const MOVE_SPEED: f32 = 5.0;
const LOOK_SPEED: f32 = 0.1;
const INTERPOLATION_STEPS: usize = 8;
const BLOCK_LEN: usize = 128;
const CHUNK: usize = INTERPOLATION_STEPS * BLOCK_LEN;

struct AudioState {
    input_tx: Sender<Vec<f32>>,
    running: Arc<AtomicBool>,
}

fn conf() -> Conf {
    Conf {
        window_title: String::from("3D Audio Demo"),
        window_width: 1280,
        window_height: 720,
        fullscreen: false,
        ..Default::default()
    }
}

fn setup_audio() -> Result<(AudioState, cpal::Stream)> {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device");
    let config = device.default_output_config()?;
    let sample_rate = config.sample_rate().0 as f32;

    // Load audio file
    let samples = hound::WavReader::open("assets/sample-0.wav")?;
    let wav_sample_rate = samples.spec().sample_rate as f32;
    let samples: Vec<f32> = match samples.spec().sample_format {
        hound::SampleFormat::Float => samples.into_samples().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => samples
            .into_samples::<i16>()
            .filter_map(Result::ok)
            .map(|s| s as f32 / i16::MAX as f32)
            .collect(),
    };

    // Resample audio
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

    // Initialize HRTF
    let hrtf_sphere = hrtf::HrirSphere::from_file("assets/hrir-3.bin", sample_rate as u32)
        .map_err(|e| anyhow::anyhow!("Failed to load HRTF: {:?}", e))?;
    let processor = hrtf::HrtfProcessor::new(hrtf_sphere, INTERPOLATION_STEPS, BLOCK_LEN);

    let (input_tx, input_rx) = mpsc::channel();
    let (output_tx, output_rx) = mpsc::channel();
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    thread::spawn(move || {
        process_audio_chunks(
            processor,
            input_rx,
            output_tx,
            sample_rate,
            running_clone,
            resampled_samples,
        );
    });

    let mut leftover: Vec<f32> = Vec::new();
    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut idx = 0;
            if !leftover.is_empty() {
                let len = leftover.len().min(data.len());
                data[..len].copy_from_slice(&leftover[..len]);
                idx += len;
                leftover.drain(..len);
            }
            while idx < data.len() {
                match output_rx.try_recv() {
                    Ok(buffer) => {
                        let len = buffer.len().min(data.len() - idx);
                        data[idx..idx + len].copy_from_slice(&buffer[..len]);
                        idx += len;
                        if len < buffer.len() {
                            leftover.extend_from_slice(&buffer[len..]);
                        }
                    }
                    Err(_) => {
                        data[idx..].fill(0.0);
                        break;
                    }
                }
            }
        },
        |err| eprintln!("Audio error: {}", err),
        None,
    )?;

    Ok((AudioState { input_tx, running }, stream))
}

fn process_audio_chunks(
    mut processor: hrtf::HrtfProcessor,
    input_rx: Receiver<Vec<f32>>,
    output_tx: Sender<Vec<f32>>,
    sample_rate: f32,
    running: Arc<AtomicBool>,
    samples: Vec<f32>,
) {
    let mut prev_left_samples = vec![];
    let mut prev_right_samples = vec![];
    let mut previous_sample_vector = hrtf::Vec3::new(0.0, 0.0, 1.0);
    let mut current_sample_vector = hrtf::Vec3::new(0.0, 0.0, 1.0);
    let prev_distance_gain = 1.0;
    let mut filter = AudioBandProcessor::new();

    // Calculate time per chunk based on sample rate
    let chunk_duration = CHUNK as f32 / sample_rate;
    let mut last_process_time = std::time::Instant::now();

    while running.load(Ordering::SeqCst) {
        // Wait until next chunk should be processed
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(last_process_time).as_secs_f32();
        if elapsed < chunk_duration {
            std::thread::sleep(std::time::Duration::from_secs_f32(chunk_duration - elapsed));
        }
        last_process_time = std::time::Instant::now();

        for chunk in samples.chunks(CHUNK) {
            if !running.load(Ordering::SeqCst) {
                break;
            }

            let mut filter_output = vec![0.0; chunk.len()];
            filter.process_buffer(chunk, &mut filter_output);

            let mut output = vec![(0.0f32, 0.0f32); chunk.len()];

            // Block waiting for position update
            if let Ok(position_data) = input_rx.recv() {
                current_sample_vector =
                    hrtf::Vec3::new(position_data[0], position_data[1], position_data[2]);
            }

            let context = hrtf::HrtfContext {
                source: &filter_output,
                output: &mut output,
                new_sample_vector: current_sample_vector, // Use current not previous
                prev_sample_vector: previous_sample_vector,
                prev_left_samples: &mut prev_left_samples,
                prev_right_samples: &mut prev_right_samples,
                new_distance_gain: prev_distance_gain,
                prev_distance_gain,
            };

            processor.process_samples(context);
            previous_sample_vector = current_sample_vector;

            let stereo_buffer: Vec<f32> = output
                .iter()
                .flat_map(|&(left, right)| vec![left, right])
                .collect();

            if output_tx.send(stereo_buffer).is_err() {
                break;
            }
        }
    }
}

#[macroquad::main(conf)]
async fn main() -> Result<()> {
    // Initialize audio
    let (audio_state, stream) = setup_audio()?;
    stream.play()?;

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let running = audio_state.running.clone();

    // Camera setup
    let world_up = vec3(0.0, 1.0, 0.0);
    let mut yaw: f32 = -90.0_f32.to_radians(); // Start looking down -Z
    let mut pitch: f32 = 0.0;

    let mut front = vec3(
        yaw.cos() * pitch.cos(),
        pitch.sin(),
        yaw.sin() * pitch.cos(),
    )
    .normalize();
    let mut right = front.cross(world_up).normalize();
    let mut up = right.cross(front).normalize();

    let mut position = vec3(0.0, 1.0, -3.0);
    let mut last_mouse_position: Vec2 = mouse_position().into();
    let mut grabbed = false;

    loop {
        let delta = get_frame_time();

        // Mouse look control
        if is_mouse_button_pressed(MouseButton::Left) {
            set_cursor_grab(true);
            show_mouse(false);
            grabbed = true;
            last_mouse_position = mouse_position().into();
        }
        if is_key_pressed(KeyCode::Escape) {
            set_cursor_grab(false);
            show_mouse(true);
            grabbed = false;
        }

        if grabbed {
            let mouse_position: Vec2 = mouse_position().into();
            let mouse_delta = mouse_position - last_mouse_position;
            last_mouse_position = mouse_position;

            yaw += mouse_delta.x * delta * LOOK_SPEED;
            pitch += mouse_delta.y * delta * -LOOK_SPEED;

            pitch = pitch.clamp(-1.5, 1.5);

            front = vec3(
                yaw.cos() * pitch.cos(),
                pitch.sin(),
                yaw.sin() * pitch.cos(),
            )
            .normalize();
            right = front.cross(world_up).normalize();
            up = right.cross(front).normalize();
        }

        // WASD movement
        if is_key_down(KeyCode::W) {
            position += front * MOVE_SPEED * delta;
        }
        if is_key_down(KeyCode::S) {
            position -= front * MOVE_SPEED * delta;
        }
        if is_key_down(KeyCode::A) {
            position -= right * MOVE_SPEED * delta;
        }
        if is_key_down(KeyCode::D) {
            position += right * MOVE_SPEED * delta;
        }
        if is_key_down(KeyCode::Space) {
            position += world_up * MOVE_SPEED * delta;
        }
        if is_key_down(KeyCode::LeftShift) {
            position -= world_up * MOVE_SPEED * delta;
        }

        // Update camera
        set_camera(&Camera3D {
            position: position,
            up: up,
            target: position + front,
            ..Default::default()
        });

        // Draw scene
        clear_background(LIGHTGRAY);

        // Draw ground grid
        draw_grid(20, 1.0, BLACK, GRAY);

        // Draw sound-emitting cube in center
        draw_cube(vec3(0.0, 0.5, 0.0), vec3(1.0, 1.0, 1.0), None, RED);

        // Calculate audio parameters based on player position
        let cube_pos = vec3(0.0, 0.5, 0.0);
        let to_cube = cube_pos - position;

        // Send spatial audio parameters as a vector
        let _ = audio_state
            .input_tx
            .send(vec![to_cube.x, to_cube.y, to_cube.z]);

        // Draw UI text
        draw_text(
            &format!(
                "Position: ({:.1}, {:.1}, {:.1})\nRotation: ({:.1}, {:.1})",
                position.x,
                position.y,
                position.z,
                yaw.to_degrees(),
                pitch.to_degrees()
            ),
            10.0,
            30.0,
            20.0,
            BLACK,
        );

        if is_key_pressed(KeyCode::Q) {
            // Shutdown signal
            running.store(false, Ordering::SeqCst);
            let _ = shutdown_tx.send(true);
            break;
        }

        next_frame().await;
    }

    Ok(())
}

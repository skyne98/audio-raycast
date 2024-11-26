// src/main.rs

use macroquad::models::draw_cube;
use macroquad::prelude::*;
use std::thread;
use tokio::runtime::Runtime;
use tokio::sync::watch;

// Constants for movement speed and mouse sensitivity
const MOVE_SPEED: f32 = 5.0;
const LOOK_SPEED: f32 = 0.1;

// Configuration for the Macroquad window
fn conf() -> Conf {
    Conf {
        window_title: String::from("Macroquad First-Person Demo with Tokio"),
        window_width: 1280,
        window_height: 720,
        fullscreen: false,
        ..Default::default()
    }
}

#[macroquad::main(conf)]
async fn main() {
    // Create a watch channel for shutdown signaling
    let (tx, mut rx) = watch::channel(false);

    // Clone the transmitter to move into the Tokio thread
    let tx_clone = tx.clone();

    // Launch Tokio runtime in a separate thread
    thread::spawn(move || {
        // Initialize Tokio runtime
        let rt = Runtime::new().expect("Failed to create Tokio runtime");

        rt.block_on(async {
            loop {
                tokio::select! {
                    // Example async task: Periodically log a message every 2 seconds
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                        println!("Tokio async task is running in the background.");
                    },
                    // Listen for shutdown signal
                    changed = rx.changed() => {
                        if changed.is_ok() && *rx.borrow() {
                            println!("Shutdown signal received. Exiting Tokio thread.");
                            break;
                        }
                    }
                }
            }
        });
    });

    // Player settings
    let world_up = vec3(0.0, 1.0, 0.0);
    let mut yaw: f32 = -90.0; // Facing towards negative Z
    let mut pitch: f32 = 0.0;

    let mut front = vec3(
        yaw.to_radians().cos() * pitch.to_radians().cos(),
        pitch.to_radians().sin(),
        yaw.to_radians().sin() * pitch.to_radians().cos(),
    )
    .normalize();

    let mut right = front.cross(world_up).normalize();
    let mut up = right.cross(front).normalize();

    let mut position = vec3(0.0, 1.0, 5.0); // Starting position

    let mut last_mouse_position: Vec2 = mouse_position().into();
    let mut grabbed = true;
    set_cursor_grab(grabbed);
    show_mouse(!grabbed);

    loop {
        let delta = get_frame_time();

        // Input handling
        if is_key_pressed(KeyCode::Escape) {
            // Send shutdown signal
            let _ = tx_clone.send(true);
            break;
        }
        if is_key_pressed(KeyCode::Tab) {
            grabbed = !grabbed;
            set_cursor_grab(grabbed);
            show_mouse(!grabbed);
        }

        // Movement
        let mut direction = Vec3::ZERO;
        if is_key_down(KeyCode::W) {
            direction += front;
        }
        if is_key_down(KeyCode::S) {
            direction -= front;
        }
        if is_key_down(KeyCode::A) {
            direction -= right;
        }
        if is_key_down(KeyCode::D) {
            direction += right;
        }
        if is_key_down(KeyCode::Space) {
            direction += world_up;
        }
        if is_key_down(KeyCode::LeftShift) {
            direction -= world_up;
        }

        if direction.length_squared() != 0.0 {
            direction = direction.normalize();
            position += direction * MOVE_SPEED * delta;
        }

        // Mouse movement
        let current_mouse_position: Vec2 = mouse_position().into();
        let mouse_delta = current_mouse_position - last_mouse_position;
        last_mouse_position = current_mouse_position;

        if grabbed {
            yaw += mouse_delta.x * LOOK_SPEED;
            pitch -= mouse_delta.y * LOOK_SPEED;

            // Constrain pitch
            if pitch > 89.0 {
                pitch = 89.0;
            }
            if pitch < -89.0 {
                pitch = -89.0;
            }

            // Update front, right, and up vectors
            front = vec3(
                yaw.to_radians().cos() * pitch.to_radians().cos(),
                pitch.to_radians().sin(),
                yaw.to_radians().sin() * pitch.to_radians().cos(),
            )
            .normalize();

            right = front.cross(world_up).normalize();
            up = right.cross(front).normalize();
        }

        // Rendering
        clear_background(LIGHTGRAY);

        // Set up camera
        set_camera(&Camera3D {
            position,
            up,
            target: position + front,
            ..Default::default()
        });

        // Draw grid for reference
        draw_grid(20, 1.0, BLACK, GRAY);

        // Draw central cube at origin
        let central_cube_position = vec3(0.0, 0.5, 0.0);
        draw_cube(central_cube_position, vec3(1.0, 1.0, 1.0), None, RED);
        draw_cube_wires(central_cube_position, vec3(1.0, 1.0, 1.0), BLACK);

        // Reset to default camera to draw UI elements
        set_default_camera();

        // UI Text
        draw_text(
            "First Person Camera with Tokio Runtime",
            10.0,
            30.0,
            30.0,
            BLACK,
        );
        draw_text(
            format!(
                "Position: X {:.1}, Y {:.1}, Z {:.1}",
                position.x, position.y, position.z
            )
            .as_str(),
            10.0,
            60.0,
            20.0,
            BLACK,
        );
        draw_text(
            format!("Yaw: {:.1}, Pitch: {:.1}", yaw, pitch).as_str(),
            10.0,
            90.0,
            20.0,
            BLACK,
        );
        draw_text(
            format!("Mouse Grabbed: {}", grabbed).as_str(),
            10.0,
            120.0,
            20.0,
            BLACK,
        );
        draw_text(
            "Controls: W/A/S/D to move, Mouse to look, Space to ascend, Shift to descend, Tab to toggle mouse grab, Esc to exit",
            10.0,
            150.0,
            20.0,
            BLACK,
        );

        next_frame().await
    }
}

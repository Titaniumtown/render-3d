// Requires nightly, allows for constant implementations of traits, for Self::Default for storage and small perf improvements.
#![feature(const_trait_impl)]

mod camera;
mod math;
mod world;

use camera::Camera;
use math::{Quat, Vec3};
use world::{Color, Transform, World};

use pixels::{Pixels, SurfaceTexture};
use rayon::prelude::*;
use winit::{
    dpi::LogicalSize,
    event::{Event, VirtualKeyCode},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use winit_input_helper::WinitInputHelper;

/// Dimentions of the Window (in pixels), width by height
const DIMS: (u32, u32) = (400, 400);

fn main() {
    let world = ron::from_str::<World>(include_str!("../scenes/sample.ron"))
        .expect("failed to parse World file");
    let mut camera = Camera {
        transform: Transform {
            position: -0.8 * Vec3::i(),
            rotation: Quat::one(),
        },
        px_per_unit: 60.0,
        focal_length: 2.0,
    };

    let event_loop = EventLoop::new();
    let mut input = WinitInputHelper::new();
    let window = {
        let size = LogicalSize::new(DIMS.0 as f64, DIMS.1 as f64);
        WindowBuilder::new()
            .with_title("Raytracing Test")
            .with_inner_size(size)
            .with_min_inner_size(size)
            .with_decorations(false) // weird graphical issue happens without this (at least on gnome + wayland) further investigation needed
            .build(&event_loop)
            .expect("WindowBuilder failed")
    };

    let mut pixels = {
        let window_size = window.inner_size();
        let surface_texture = SurfaceTexture::new(window_size.width, window_size.height, &window);
        Pixels::new(DIMS.0, DIMS.1, surface_texture).expect("failed to create pixels")
    };

    queue_render(pixels.get_frame_mut(), &world, &camera);

    let mut last_frame = std::time::Instant::now();

    event_loop.run(move |event, _, control_flow| {
        let mut this_frame = std::time::Instant::now();
        let mut delta_time = this_frame - last_frame;
        let min_frame_time = std::time::Duration::from_millis(10);
        if delta_time < min_frame_time {
            std::thread::sleep(min_frame_time - delta_time);
            delta_time = min_frame_time;
            this_frame = last_frame + delta_time;
        }
        let delta_time = delta_time.as_secs_f32();
        let keyboard_input: bool = input.update(&event) && {
            if (input.key_held(VirtualKeyCode::LControl)
                || input.key_held(VirtualKeyCode::RControl))
                && input.key_pressed(VirtualKeyCode::C)
            {
                *control_flow = ControlFlow::Exit;
            }

            const MOVE_SPEED: f32 = 3.0;
            const TURN_SPEED: f32 = std::f32::consts::FRAC_PI_2;
            let mut did_movement: bool = false;

            let mut movement = |delta: Vec3| {
                camera.transform.position += delta.rotate(camera.transform.rotation) * delta_time;
                did_movement = true;
            };

            if input.key_held(VirtualKeyCode::W) {
                movement(MOVE_SPEED * Vec3::i());
            }
            if input.key_held(VirtualKeyCode::S) {
                movement(-MOVE_SPEED * Vec3::i());
            }
            if input.key_held(VirtualKeyCode::A) {
                movement(MOVE_SPEED * Vec3::j());
            }
            if input.key_held(VirtualKeyCode::D) {
                movement(-MOVE_SPEED * Vec3::j());
            }
            if input.key_held(VirtualKeyCode::E) {
                movement(MOVE_SPEED * Vec3::k());
            }
            if input.key_held(VirtualKeyCode::Q) {
                movement(-MOVE_SPEED * Vec3::k());
            }
            if input.key_held(VirtualKeyCode::X) {
                movement(MOVE_SPEED * Vec3::i());
                camera.focal_length -= MOVE_SPEED * delta_time;
            }
            if input.key_held(VirtualKeyCode::Z) {
                movement(-MOVE_SPEED * Vec3::i());
                camera.focal_length += MOVE_SPEED * delta_time;
            }
            if input.key_held(VirtualKeyCode::R) {
                camera.focal_length += MOVE_SPEED * delta_time;
                did_movement = true;
            }
            if input.key_held(VirtualKeyCode::F) {
                camera.focal_length -= MOVE_SPEED * delta_time;
                did_movement = true;
            }

            let mut did_rotation: bool = false;

            let mut rotation = |angle: f32, axis: Vec3| {
                let angle = angle * delta_time;
                let hf_angle = angle / 2.0;
                let new_rot = hf_angle.cos() + axis * hf_angle.sin();

                let rot = &mut camera.transform.rotation;
                let new_rot = *rot * new_rot * rot.conj();

                // Mathematically, the magnitude should always remain at 1 already, but floating point
                // precision errors cause self-fueleing inaccuracy that becomes worse with each rotation.
                let new_rot = new_rot * new_rot.mag().recip();
                *rot = new_rot * *rot;
                did_rotation = true;
            };

            if input.key_held(VirtualKeyCode::J) {
                rotation(TURN_SPEED, Vec3::k());
            }
            if input.key_held(VirtualKeyCode::L) {
                rotation(-TURN_SPEED, Vec3::k());
            }
            if input.key_held(VirtualKeyCode::K) {
                rotation(TURN_SPEED, Vec3::j());
            }
            if input.key_held(VirtualKeyCode::I) {
                rotation(-TURN_SPEED, Vec3::j());
            }
            if input.key_held(VirtualKeyCode::O) {
                rotation(TURN_SPEED, Vec3::i());
            }
            if input.key_held(VirtualKeyCode::U) {
                rotation(-TURN_SPEED, Vec3::i())
            }

            did_rotation || did_movement
        };

        let redraw_requested: bool = matches!(event, Event::RedrawRequested(_));

        // Draw the current frame
        if keyboard_input || redraw_requested {
            queue_render(pixels.get_frame_mut(), &world, &camera);
            if pixels
                .render()
                .map_err(|e| panic!("pixels.render() failed: {}", e))
                .is_err()
            {
                *control_flow = ControlFlow::Exit;
            }
        }
        last_frame = this_frame;
    });
}

fn queue_render(frame: &mut [u8], world: &World, camera: &Camera) {
    // Create a instant here to time how long it takes to render a frame
    let now = std::time::Instant::now();

    // Used to zip with frame data in place of enumerating (which cannot be done with par_chunks_exact_mut)
    let index = 0..(DIMS.0 * DIMS.1);

    frame
        .par_chunks_exact_mut(4)
        .zip(index)
        .for_each(|(pixel, i)| {
            // (x, y) of pixel on screen
            let (x, y): (i32, i32) = (((i % DIMS.0) as i32), ((i / DIMS.0) as i32));
            let x_w = x as f32 - (DIMS.0 as f32) / 2.0;
            let y_w = y as f32 - (DIMS.1 as f32) / 2.0;

            let rgb: Color = camera.get_px(world, x_w, y_w);
            let rgba: [u8; 4] = [rgb[0], rgb[1], rgb[2], 255];

            pixel.copy_from_slice(&rgba);
        });

    // TODO: Add toggleable debug overlay with this information
    eprintln!("Frame took: {}ms", now.elapsed().as_millis());
}

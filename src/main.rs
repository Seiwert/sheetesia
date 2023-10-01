#[macro_use]
extern crate lazy_static;

use opencv::core::*;
use opencv::imgcodecs::*;
use opencv::imgproc::LINE_8;
use opencv::videoio::*;
use opencv::imgproc;
use opencv::highgui::*;

use ghakuf::messages::*;
use ghakuf::writer::*;
use std::path;

mod piano;
use crate::piano::piano::*;

lazy_static! {
	static ref UNCHANGED: VecN<f64, 4> = {
		Scalar::new(100f64,100f64,100f64,255f64)
	};
	static ref PRESSED: VecN<f64, 4> = {
		Scalar::new(0f64,255f64,255f64,255f64)
	};
	static ref RELEASED: VecN<f64, 4> = {
		Scalar::new(0f64,0f64,255f64,255f64)
	};
}
const DEBUG: bool = false;
fn main() {
	// Load files
	let video_path = std::env::args().nth(1).expect("Please provide a video as an argument");
	let bpm: u32 = std::env::args().nth(2).unwrap_or("120".into()).parse().expect("Please provide an integer bpm if supplied");
	let mut video: VideoCapture = VideoCapture::from_file(&video_path, CAP_ANY).unwrap();
	
	let template: Mat = imread("res/template-partial.png", IMREAD_COLOR).unwrap();

	if !video.is_opened().unwrap() || template.empty() {
		println!(":(");
		std::process::exit(1);
	}
	println!("Loaded video: {}", video_path);

	// Make window
	named_window("Gabo", WINDOW_AUTOSIZE).unwrap();

	// ----------------
	// READ FIRST FRAME
	let mut frame: Mat = Mat::default();
	// Find piano
	let mut piano:Piano;
	println!("Finding piano...");
	// Loop until we find the piano
	loop {
		video.read(&mut frame).unwrap();
		let result= Piano::new(&frame, &template);
		match result {
			Some(piano_result) => {
				if piano_result.octaves.len() < 4{
					println!("Only found {} octaves, continueing", piano_result.octaves.len());
					continue;
				}
				else {
					piano = piano_result;
					for octave in &mut piano.octaves {
						for note in &mut octave.notes {
							imgproc::circle(&mut frame, Point {
								x: note.location.x,
								y: note.location.y,
							}, 10, UNCHANGED.clone(),-1,LINE_8, 0).unwrap();
						}
					}
					imshow("initial", &frame).unwrap();
					let key_pressed = wait_key(0).unwrap();
					if key_pressed == 113 { // Space bar pressed
						break;
					}
					else if key_pressed == 110 { // N key pressed. Manually reject detection to continue
						continue;
					}
				}
				break;
			}
			None => { continue; }
		}
	}
	println!("Piano has been found!");
	
	// ----------
	// PLAY VIDEO
	println!("Detecting notes!");

	// MIDI variables
	let path = path::Path::new("out.mid");
	let beats_per_second = (bpm as f64)/60f64;
	let quarter_note_resolution = 480;
	let microseconds_per_quarter_note: u32 = 60*1000000/bpm; // set bpm to 120
	let ticks_per_frame: u32 = ((beats_per_second*quarter_note_resolution as f64)/video.get(CAP_PROP_FPS).unwrap()) as u32;
	println!("ticks_per_frame={}", video.get(CAP_PROP_FPS).unwrap());
	// MIDI messages
	let mut midi_messages: Vec<Message> = vec![
		// Set video FPS as tempo
		Message::MetaEvent {
			delta_time: 0,
			event: MetaEvent::SetTempo,
			data: [(microseconds_per_quarter_note >> 16) as u8, (microseconds_per_quarter_note >> 8) as u8, microseconds_per_quarter_note as u8].to_vec(),
		},
	
		Message::MetaEvent {
			delta_time: 0,
			event: MetaEvent::EndOfTrack,
			data: Vec::new(),
		},
	
		Message::TrackChange
	];

	// Vector storing each note and the color it had on the previous frame
	//	Initialize with each note's default color
	let mut previous_frame_note_colors: Vec<Vec<Vec3b>> = Vec::new();
	for octave in &piano.octaves {
		let mut note_colors_vec: Vec<Vec3b> = Vec::new();
		for note in &octave.notes {
			note_colors_vec.push(note.default_color);
		}
		previous_frame_note_colors.push(note_colors_vec);
	}

	// Video variables
	let total_frames = video.get(CAP_PROP_FRAME_COUNT).unwrap();
	let mut frame_count: u32 = 0; // number of played frames
	let mut frame_count_on_last_event: u32 = 0; // because SMF uses time since last event rather than absolute time

	// LOOP UNTIL THE VIDEO ENDS
	let between_frames_thresh: i32 = 150;
	let color_thresh: i32 = 200;

	loop {
		if !video.read(&mut frame).unwrap() {
			println!("Done");
			break;
		}
		
		let mut key_pressed_or_released_in_this_frame = false;

		// For each note in every octave
		let mut octave_index = 0;
		for octave in &mut piano.octaves {
			let mut note_index = 0;
			for note in &mut octave.notes {
				let note_color: Vec3b = *frame.at_2d(note.location.y, note.location.x).unwrap();
				
				// Skip check if the color is close to the one in the previous frame
				let mut diff_with_previous_frame: i32 = 0;
				for i in 0..3 {
					diff_with_previous_frame += (note_color[i] as i32 - previous_frame_note_colors[octave_index][note_index][i] as i32).abs();
				}
				let mut color: VecN<f64, 4> = UNCHANGED.clone(); 

				// If the color isn't close to the one in the previous frame
				//	Check if the color is close to the default color (Key released) or not (key pressed)
				if diff_with_previous_frame > between_frames_thresh {	
					let mut diff_with_default_color: i32 = 0;
					for i in 0..3 {
						diff_with_default_color += (note_color[i] as i32 - note.default_color[i] as i32).abs();
					}

					let result: Result<bool, bool> = note.set_pressed(diff_with_default_color > color_thresh);
					match result {
						Ok(pressed) => {
							if pressed {
								println!("{}\tpressed \tat second: {} \t@ frame {} of {}\t({:.2}%)", note.to_string(), (frame_count*ticks_per_frame)/1000, frame_count, total_frames, (frame_count as f64 / total_frames)*100.0);
								midi_messages.push(
									Message::MidiEvent {
										delta_time: (frame_count - frame_count_on_last_event) * ticks_per_frame,
										event: MidiEvent::NoteOn {
											ch: 0,
											note: note.code,
											velocity: 0x7f,
										},
									}
								);
								color = PRESSED.clone();
							} else {
								println!("{}\treleased \tat second: {} \t@ frame {} of {}\t({:.2}%)", note.to_string(), (frame_count*ticks_per_frame)/1000, frame_count, total_frames, (frame_count as f64 / total_frames)*100.0);
								midi_messages.push(
									Message::MidiEvent {
										delta_time: (frame_count - frame_count_on_last_event) * ticks_per_frame,
										event: MidiEvent::NoteOff {
											ch: 0,
											note: note.code,
											velocity: 0x7f,
										},
									}
								);
								color = RELEASED.clone();
							}
							// Show frame
							key_pressed_or_released_in_this_frame = true;

							// Update variable to keep track of the last pushed MIDI event
							frame_count_on_last_event = frame_count;
						},
						Err(_) => {}
					};
				}
				if DEBUG {
					imgproc::circle(&mut frame, Point {
						x: note.location.x,
						y: note.location.y,
					}, 10, color,-1,LINE_8, 0).unwrap();
				}
				// Update previous frame pixel color
				previous_frame_note_colors[octave_index][note_index] = note_color;
				note_index = note_index+1;
			}

			octave_index = octave_index+1;
		}

		// Show frame
		if key_pressed_or_released_in_this_frame {
			if DEBUG {
				imshow("Gabo", &frame).unwrap();
				let key_pressed = wait_key(if DEBUG { 10000 } else {1}).unwrap();
				if key_pressed == 113 {
					break;
				}
			}
		}

		// Increment frame count
		frame_count = frame_count + 1;
	}
	

	// Ending MIDI messages
	midi_messages.push(
		Message::MetaEvent {
			delta_time: 0,
			event: MetaEvent::EndOfTrack,
			data: Vec::new(),
    	}
	);

	// Write MIDI
	let mut midi_writer = Writer::new();
	midi_writer.running_status(true);
	for message in &midi_messages {
    	midi_writer.push(message);
	}
	midi_writer.write(&path).unwrap();

	// Cleanup
	video.release().unwrap();
	destroy_window("Gabo").unwrap();
}

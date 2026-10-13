//! The viewport camera as projects keep it: the project's
//! `Viewport.CameraPosition` (the `SetCamera` message the viewport is
//! sent: 32, then six floats) and camera files (`.cam`: the viewport's
//! `camera|…` reply, one value a line).

use nerthus_engine::automation::dotnet_float_string;
use nerthus_render::CameraValues;

/// `SetCamera`'s message code.
const SET_CAMERA: u8 = 32;

/// The values in the order they are sent and saved (`GetValues`).
fn ordered(values: &CameraValues) -> [f32; 6] {
    [values.look_at.x, values.look_at.y, values.look_at.z, values.inclination, values.zoom, values.rotation]
}

fn from_ordered(numbers: [f32; 6]) -> CameraValues {
    let [x, y, z, inclination, zoom, rotation] = numbers;
    CameraValues {
        look_at: bevy::math::Vec3::new(x, y, z),
        inclination,
        zoom,
        rotation,
    }
}

/// `ConvertLinesToCamera`'s message: the code, then each value's bytes.
pub fn camera_message(values: &CameraValues) -> Vec<u8> {
    let mut message = vec![SET_CAMERA];
    for number in ordered(values) {
        message.extend_from_slice(&number.to_le_bytes());
    }
    message
}

/// The camera in a saved message, if it holds one (`ResetCamera` sends
/// whatever is saved; an empty list does nothing).
pub fn camera_from_message(message: &[u8]) -> Option<CameraValues> {
    if message.len() < 25 || message[0] != SET_CAMERA {
        return None;
    }
    let mut numbers = [0.0; 6];
    for (index, number) in numbers.iter_mut().enumerate() {
        let start = 1 + index * 4;
        let bytes = [message[start], message[start + 1], message[start + 2], message[start + 3]];
        *number = f32::from_le_bytes(bytes);
    }
    Some(from_ordered(numbers))
}

/// A camera file's lines (`SaveCamera`: the reply split at `|`).
pub fn camera_file_lines(values: &CameraValues) -> Vec<String> {
    let mut lines = vec!["camera".to_string()];
    for number in ordered(values) {
        lines.push(dotnet_float_string(number));
    }
    lines
}

/// The camera a file's lines hold (`ConvertLinesToCamera` reads lines 1
/// to 6).
pub fn camera_from_lines(lines: &[&str]) -> Result<CameraValues, String> {
    let mut numbers = [0.0; 6];
    for (index, number) in numbers.iter_mut().enumerate() {
        let line = lines.get(index + 1).ok_or("the camera file is too short")?;
        *number = line.trim().parse::<f32>().map_err(|_| format!("\"{line}\" is not a number"))?;
    }
    Ok(from_ordered(numbers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_like_the_examples() {
        // A saved camera from one of the example projects.
        let values = CameraValues {
            look_at: bevy::math::Vec3::new(0.8422, 0.0, -1.7633),
            inclination: 0.6175,
            zoom: 47.985,
            rotation: -0.128,
        };
        let message = camera_message(&values);
        assert_eq!(message.len(), 25);
        assert_eq!(message[0], 32);
        assert_eq!(camera_from_message(&message), Some(values));
        assert_eq!(camera_from_message(&[]), None);

        let default = camera_message(&CameraValues::DEFAULT);
        assert_eq!(&default[13..17], &0.3f32.to_le_bytes());
        assert_eq!(&default[17..21], &50.0f32.to_le_bytes());
    }

    #[test]
    fn files_hold_a_value_a_line() {
        let lines = camera_file_lines(&CameraValues::DEFAULT);
        assert_eq!(lines, ["camera", "0", "0", "0", "0.3", "50", "0"]);
        let read: Vec<&str> = lines.iter().map(String::as_str).collect();
        assert_eq!(camera_from_lines(&read), Ok(CameraValues::DEFAULT));
        assert!(camera_from_lines(&["camera", "1"]).is_err());
    }
}

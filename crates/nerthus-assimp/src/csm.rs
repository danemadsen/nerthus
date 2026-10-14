//! Assimp's CharacterStudio Motion importer (`CSMLoader.cpp`): `$`
//! sections (`$firstframe`, `$lastframe`, `$rate`, `$order` naming the
//! markers, `$points` with a frame number and x y z per marker, or
//! `DROPOUT`). Each marker becomes a node at its first sample under a
//! dummy root, and the mesh is `SkeletonMeshBuilder`'s knobs at them.
//!
//! The key arrays are sized from `$lastframe` (no `$lastframe` leaves them
//! null), and growing one sets its key count to the new size, so the next
//! key lands past it: reads and writes outside the arrays, and the endless
//! `$order` loop at a NUL or form feed, are errors here.

use crate::fast_atof::{fast_atoreal_move, strtol10, strtoul10};
use crate::matrix::Matrix4;
use crate::node::Node;
use crate::parsing::{at, is_line_end, is_space_or_newline, skip_line, skip_spaces, skip_spaces_and_line_end, text_file_to_buffer, token_match_i};
use crate::scene::{ImportError, Scene, check_allocation, fail};
use crate::skeleton;

/// `sizeof(aiVectorKey)`.
const KEY_SIZE: u64 = 24;

/// `AI_MAXLEN`: a name this long overflows its `aiString`.
const MAX_NAME_LENGTH: usize = 1024;

/// The default `last` frame, meaning no `$lastframe`.
const NO_LAST_FRAME: i32 = 0x00ff_ffff;

/// A marker's `aiNodeAnim`: its position key array (null until sized) and
/// key count.
struct Channel {
    keys: Option<Vec<[f32; 3]>>,
    count: u32,
}

/// Blanks and line ends skipped before a coordinate, which must follow.
fn next_coordinate(buffer: &[u8], position: &mut usize, end: usize, axis: &str) -> Result<f32, ImportError> {
    let (next, goes_on) = skip_spaces_and_line_end(buffer, *position, end);
    *position = next;
    if !goes_on {
        return fail(format!("CSM: Unexpected EOF occurred reading sample {axis} coord"));
    }
    let (value, next) = fast_atoreal_move(buffer, *position, true)?;
    *position = next;
    Ok(value)
}

/// The `$order` section: one channel per name until a line ends before a
/// `$`.
fn read_order(buffer: &[u8], position: &mut usize, end: usize) -> Result<Vec<Channel>, ImportError> {
    let mut channels = Vec::new();
    loop {
        *position = skip_spaces(buffer, *position, end).0;
        if is_line_end(at(buffer, *position)) {
            let (next, goes_on) = skip_spaces_and_line_end(buffer, *position, end);
            *position = next;
            if goes_on && at(buffer, *position) == b'$' {
                break;
            }
        }
        let name_start = *position;
        while !is_space_or_newline(at(buffer, *position)) {
            *position += 1;
        }
        let name_length = *position - name_start;
        if name_length == 0 {
            // A NUL or form feed here never ends the section: channels are
            // added until memory runs out.
            return fail("Internal error");
        }
        if name_length >= MAX_NAME_LENGTH {
            return fail("undefined behaviour: a marker name overflows its aiString");
        }
        channels.push(Channel { keys: None, count: 0 });
    }
    if channels.is_empty() {
        return fail("CSM: Empty $order section");
    }
    Ok(channels)
}

/// Doubles a channel's key array when its count reached `alloc`, the
/// count becoming the new size.
fn grow_keys(channel: &mut Channel, alloc: u32) -> Result<(), ImportError> {
    let new_size = alloc.wrapping_mul(2);
    check_allocation(new_size as u64, KEY_SIZE)?;
    let Some(old) = channel.keys.take() else {
        return fail("undefined behaviour: a copy from a null key array");
    };
    if (old.len() as u64) < alloc as u64 {
        return fail("undefined behaviour: a copy past a key array");
    }
    let mut keys = old[..alloc as usize].to_vec();
    keys.resize(new_size as usize, [0.0; 3]);
    channel.keys = Some(keys);
    channel.count = new_size;
    Ok(())
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let buffer = text_file_to_buffer(bytes, false)?;
    let end = buffer.len();
    let mut first: i32 = 0;
    let mut last: i32 = NO_LAST_FRAME;
    let mut channels: Vec<Channel> = Vec::new();
    let mut position = 0;

    loop {
        position = skip_spaces(&buffer, position, end).0;
        if at(&buffer, position) == 0 {
            break;
        }
        if buffer[position] != b'$' {
            position = skip_line(&buffer, position, end).0;
            continue;
        }
        position += 1;
        if token_match_i(&buffer, &mut position, b"firstframe")? {
            position = skip_spaces(&buffer, position, end).0;
            let (value, next) = strtol10(&buffer, position);
            first = value;
            position = next;
        } else if token_match_i(&buffer, &mut position, b"lastframe")? {
            position = skip_spaces(&buffer, position, end).0;
            let (value, next) = strtol10(&buffer, position);
            last = value;
            position = next;
        } else if token_match_i(&buffer, &mut position, b"rate")? {
            position = skip_spaces(&buffer, position, end).0;
            let (_ticks_per_second, next) = fast_atoreal_move(&buffer, position, true)?;
            position = next;
        } else if token_match_i(&buffer, &mut position, b"order")? {
            channels = read_order(&buffer, &mut position, end)?;
        } else if token_match_i(&buffer, &mut position, b"points")? {
            if channels.is_empty() {
                return fail("CSM: '$order' section is required to appear prior to '$points'");
            }
            let mut alloc: u32 = 100;
            if last != NO_LAST_FRAME {
                alloc = last.wrapping_sub(first) as u32;
                alloc = alloc.wrapping_add(alloc >> 2);
                for channel in &mut channels {
                    check_allocation(alloc as u64, KEY_SIZE)?;
                    channel.keys = Some(vec![[0.0; 3]; alloc as usize]);
                }
            }

            let mut filled: u32 = 0;
            loop {
                position = skip_spaces(&buffer, position, end).0;
                if is_line_end(at(&buffer, position)) {
                    let (next, goes_on) = skip_spaces_and_line_end(&buffer, position, end);
                    position = next;
                    if !goes_on || at(&buffer, position) == b'$' {
                        break;
                    }
                }
                let (frame, next) = strtoul10(&buffer, position);
                position = next;
                let frame = frame as i32;
                last = frame.max(last);
                first = frame.min(last);
                for channel in &mut channels {
                    if channel.count == alloc {
                        grow_keys(channel, alloc)?;
                    }
                    let (next, goes_on) = skip_spaces_and_line_end(&buffer, position, end);
                    position = next;
                    if !goes_on {
                        return fail("CSM: Unexpected EOF occurred reading sample x coord");
                    }
                    if token_match_i(&buffer, &mut position, b"DROPOUT")? {
                        // Invalid marker data: no key for this frame.
                        continue;
                    }
                    let slot = channel.count as usize;
                    let in_range = channel.keys.as_ref().is_some_and(|keys| slot < keys.len());
                    if !in_range {
                        return fail("undefined behaviour: a key written outside its array");
                    }
                    let (x, next) = fast_atoreal_move(&buffer, position, true)?;
                    position = next;
                    let y = next_coordinate(&buffer, &mut position, end, "y")?;
                    let z = next_coordinate(&buffer, &mut position, end, "z")?;
                    if let Some(keys) = channel.keys.as_mut() {
                        keys[slot] = [x, y, z];
                    }
                    channel.count += 1;
                }
                if filled == alloc {
                    alloc = alloc.wrapping_mul(2);
                }
                filled = filled.wrapping_add(1);
            }
            if channels.iter().any(|channel| channel.count == 0) {
                return fail("CSM: Invalid marker track");
            }
        }
    }

    // A dummy root with a node per marker at its first key.
    let mut nodes = vec![Node::new(b"$CSM_DummyRoot".to_vec(), Matrix4::IDENTITY, None)];
    for channel in &channels {
        let Some(first_key) = channel.keys.as_ref().and_then(|keys| keys.first()) else {
            return fail("undefined behaviour: a marker without a key array");
        };
        let index = nodes.len();
        nodes.push(Node::new(Vec::new(), Matrix4::translation(*first_key), Some(0)));
        nodes[0].children.push(index);
    }
    let mesh = skeleton::build_mesh(&nodes, 0, true);
    Ok(Scene { meshes: vec![mesh] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_become_knobs_at_their_first_keys() {
        let text = b"$Filename x.csm\n$FirstFrame 1\n$LastFrame 3\n$Rate 30\n$Order\nA B\n$Points\n\
            1 1 2 3 4 5 6\n2 DROPOUT 7 8 9\n";
        let scene = read(text).unwrap();
        let mesh = &scene.meshes[0];
        assert_eq!(mesh.vertex_count, 72);
        let vertices = mesh.vertices.as_ref().unwrap();
        assert_eq!(vertices[0], [0.0, 0.0, 0.0]);
    }

    #[test]
    fn keys_without_a_last_frame_are_written_through_null() {
        let text = b"$Order\nA\n$Points\n1 1 2 3\n";
        assert!(read(text).is_err());
    }

    #[test]
    fn a_section_ending_the_file_reads_past_it() {
        assert!(read(b"$Order\nA\n$points").is_err());
    }
}

//! LightWave envelopes (`LWO::Envelope`) and `LWO::AnimResolver` as the
//! scene importer uses them: a node's bind pose from its envelopes' first
//! keys, and the construction of its animation channel, which matters
//! here only for whether a channel is made and how its making can fail.
//! The range setup walks out of the key lists in almost every case where
//! it does anything, and the key merging never ends once a step cannot
//! move on (NaN times).

use crate::cast::to_unsigned;
use crate::matrix::Matrix4;
use crate::scene::{ImportError, check_allocation, fail};

/// `sizeof(LWO::Key)`.
pub(crate) const KEY_SIZE: u64 = 40;

/// `PrePostBehaviour`s.
const BEHAVIOUR_CONSTANT: u32 = 1;
const BEHAVIOUR_REPEAT: u32 = 2;
const BEHAVIOUR_OSCILLATE: u32 = 3;
const BEHAVIOUR_OFFSET_REPEAT: u32 = 4;

/// `EnvelopeType_Unknown`: types past it are out of range.
const ENVELOPE_TYPE_UNKNOWN: i32 = 16;

/// The nine envelope types that drive a node (`EnvelopeType_Position_X` to
/// `EnvelopeType_Scaling_Z`): translation, rotation (heading, pitch,
/// bank) and scaling, three each.
const COMPONENTS: usize = 9;

/// `LWO::Key`, as far as the importer's use of it goes (interpolation
/// types and their parameters change no outcome here).
#[derive(Clone, Copy, Default)]
pub(crate) struct Key {
    pub time: f64,
    pub value: f32,
}

/// `LWO::Envelope` (its post behaviour changes no outcome here).
#[derive(Clone)]
pub(crate) struct Envelope {
    /// `EnvelopeType`, as the scene file's channel number makes it.
    pub kind: u32,
    pub pre: u32,
    pub keys: Vec<Key>,
}

impl Envelope {
    pub fn new(kind: u32) -> Self {
        Envelope { kind, pre: BEHAVIOUR_CONSTANT, keys: Vec::new() }
    }
}

/// `AnimResolver` over a node's envelopes.
pub(crate) struct Resolver<'a> {
    envelopes: &'a mut [Envelope],
    /// Per component, the last envelope of its type that has keys.
    components: [Option<usize>; COMPONENTS],
}

impl<'a> Resolver<'a> {
    /// The constructor: the components found, and their key times turned
    /// from seconds into ticks.
    pub fn new(envelopes: &'a mut [Envelope], tick: f64) -> Self {
        let mut components = [None; COMPONENTS];
        for (index, envelope) in envelopes.iter_mut().enumerate() {
            if envelope.keys.is_empty() {
                continue;
            }
            let kind = envelope.kind as i32;
            if !(1..=ENVELOPE_TYPE_UNKNOWN).contains(&kind) || kind as usize > COMPONENTS {
                continue;
            }
            components[kind as usize - 1] = Some(index);
            for key in &mut envelope.keys {
                key.time *= tick;
            }
        }
        Resolver { envelopes, components }
    }

    /// The first key's value of a component's envelope.
    fn first_value(&self, component: usize, default: f32) -> f32 {
        match self.components[component] {
            Some(index) => self.envelopes[index].keys[0].value,
            None => default,
        }
    }

    /// `ExtractBindPose`: translation × heading (about y) × pitch (about
    /// x) × bank (about z) × scaling, from the first keys.
    pub fn bind_pose(&self) -> Matrix4 {
        if self.envelopes.is_empty() {
            return Matrix4::IDENTITY;
        }
        let translation = [self.first_value(0, 0.0), self.first_value(1, 0.0), self.first_value(2, 0.0)];
        let angles = [self.first_value(3, 0.0), self.first_value(4, 0.0), self.first_value(5, 0.0)];
        let scaling = [self.first_value(6, 1.0), self.first_value(7, 1.0), self.first_value(8, 1.0)];

        let rotation_z = Matrix4::rotation_z(angles[2]);
        let rotation_x = Matrix4::rotation_x(angles[1]);
        let rotation_y = Matrix4::rotation_y(angles[0]);
        let mut pose = Matrix4::translation(translation);
        pose.multiply_assign(&rotation_y);
        pose.multiply_assign(&rotation_x);
        pose.multiply_assign(&rotation_z);
        pose.multiply_assign(&Matrix4::scaling(scaling));
        pose
    }

    /// Whether a component group's envelopes have more than one key.
    fn animated(&self, group: usize) -> bool {
        let start = group * 3;
        self.components[start..start + 3].iter().flatten().any(|&index| self.envelopes[index].keys.len() > 1)
    }

    /// The keys of a component group's envelopes; a missing one stands in
    /// as a single key at time 0.
    fn group_keys(&self, group: usize) -> [Vec<Key>; 3] {
        let start = group * 3;
        let keys = |component: usize| match self.components[component] {
            Some(index) => self.envelopes[index].keys.clone(),
            None => vec![Key::default()],
        };
        [keys(start), keys(start + 1), keys(start + 2)]
    }

    /// `SetAnimationRange(first, …)`, then `ExtractAnimChannel`: whether a
    /// channel is made (some component's envelope has more than one key),
    /// after a second range setup and the key merging of each animated
    /// group.
    pub fn extract_channel(&mut self, first: f64) -> Result<bool, ImportError> {
        update_range_setup(self.envelopes, first)?;
        if self.envelopes.is_empty() {
            return Ok(false);
        }
        let animated = [self.animated(0), self.animated(1), self.animated(2)];
        if !animated.contains(&true) {
            return Ok(false);
        }
        update_range_setup(self.envelopes, first)?;
        for (group, &is_animated) in animated.iter().enumerate() {
            if !is_animated {
                continue;
            }
            let [x, y, z] = self.group_keys(group);
            merge_keys(&x, &y, &z)?;
        }
        Ok(true)
    }
}

/// `UpdateAnimRangeSetup`: for each envelope that repeats or oscillates
/// before its first key, the keys after the range start's place copied in
/// front of it and whole copies of the keys added after those, then the
/// copies' times moved back. That last walk starts near the list's end
/// and moves one key further per copy than a copy is long, so it leaves
/// the list (undefined behaviour here) unless no copy is made and keys
/// were put in front; with keys running backwards in time it never ends.
fn update_range_setup(envelopes: &mut [Envelope], first: f64) -> Result<(), ImportError> {
    for envelope in envelopes.iter_mut() {
        let pre = envelope.pre;
        let keys = &mut envelope.keys;
        let (Some(&front), Some(&back)) = (keys.first(), keys.last()) else {
            continue;
        };
        let delta = back.time - front.time;
        if delta == 0.0 {
            continue;
        }
        let old_size = keys.len();
        let value_delta = back.value - front.value;
        if !matches!(pre, BEHAVIOUR_REPEAT | BEHAVIOUR_OSCILLATE | BEHAVIOUR_OFFSET_REPEAT) {
            continue;
        }

        // The keys from the first one before the start time on, copied in
        // front.
        let start_time = delta - (front.time - first) % delta;
        let mut offset = 0usize;
        if let Some(found) = keys.iter().position(|key| start_time > key.time) {
            offset = old_size - found;
            check_allocation((old_size + offset) as u64, KEY_SIZE)?;
            let tail = keys[found..].to_vec();
            keys.splice(0..0, tail);
        }

        // Whole copies after them, every other one reversed (but for its
        // last key) when oscillating.
        let copies = to_unsigned((front.time - first) / delta);
        let added = copies as u64 * old_size as u64;
        let new_size = keys.len() as u64 + added;
        check_allocation(new_size, KEY_SIZE)?;
        keys.resize(new_size as usize, Key::default());
        let mut reverse = false;
        for copy in 0..copies as usize {
            let target = offset + old_size * (copy + 1);
            keys.copy_within(offset..offset + old_size, target);
            reverse = !reverse;
            if pre == BEHAVIOUR_OSCILLATE && reverse {
                keys[target..target + old_size - 1].reverse();
            }
        }

        // The time update, walking back from before the last copy.
        let step = old_size as i64 + 1;
        let mut position = keys.len() as i64 - step;
        let mut moved_back = delta;
        let mut copy_number: u32 = 1;
        let limit = delta * copies.wrapping_add(1) as f64;
        while moved_back <= limit {
            let stop = if delta == limit { 0 } else { position - step };
            let counts_on = delta > 0.0 && moved_back + delta != moved_back;
            if stop == position && !counts_on {
                return fail("endless loop: an envelope's repeated keys moved back forever");
            }
            while stop != position {
                let Some(key) = usize::try_from(position).ok().and_then(|index| keys.get_mut(index)) else {
                    return fail("undefined behaviour: an envelope's key times moved outside its keys");
                };
                key.time -= moved_back;
                if pre == BEHAVIOUR_OFFSET_REPEAT {
                    key.value += copy_number as f32 * value_delta;
                }
                position -= 1;
            }
            moved_back += delta;
            copy_number = copy_number.wrapping_add(1);
        }
    }
    Ok(())
}

/// `GetKeys`, as far as it can fail: its walk over the three envelopes at
/// once, which repeats forever once a step leaves every position as it
/// was. (`ExtractAnimChannel` hands it its own flags, never set, not the
/// sampling the scene importer asks for, so it reserves room by key
/// counts, never failing.)
fn merge_keys(x: &[Key], y: &[Key], z: &[Key]) -> Result<(), ImportError> {
    let mut current = [0usize; 3];
    let mut ended = [false; 3];
    let lists = [x, y, z];
    // Moves a list on, or marks the end of it.
    let advance = |current: &mut [usize; 3], ended: &mut [bool; 3], list: usize, mark: usize| {
        if current[list] != lists[list].len() - 1 {
            current[list] += 1;
        } else {
            ended[mark] = true;
        }
    };
    loop {
        let before = (current, ended);
        let times = [x[current[0]].time, y[current[1]].time, z[current[2]].time];
        let time;
        if times[0] == times[1] && times[0] == times[2] {
            time = times[0];
        } else {
            // The lowest time, as the C++ picks it (both of its branches
            // for a second list that goes on take that list's time).
            time = if times[0] <= times[1] && !ended[0] {
                if times[2] <= times[0] && !ended[2] { times[2] } else { times[0] }
            } else if !ended[1] {
                times[1]
            } else if ended[0] {
                times[2]
            } else {
                times[0]
            };
            // `InterpolateTrack`: the lists at that time move on (the last
            // one marking the first list's end when it ends).
            for (list, mark) in [(0, 0), (1, 1), (2, 0)] {
                if lists[list][current[list]].time == time {
                    advance(&mut current, &mut ended, list, mark);
                }
            }
        }
        for list in 0..3 {
            if time >= lists[list][current[list]].time {
                advance(&mut current, &mut ended, list, list);
            }
        }
        if ended == [true; 3] {
            return Ok(());
        }
        if (current, ended) == before {
            return fail("endless loop: animation keys merged forever");
        }
    }
}

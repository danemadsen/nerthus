//! The viewport camera: `StrategyCam` on the viewport's Main Camera, with
//! the settings its scene gives it and those the editor sends from the
//! user's options (`SetupMouseInput`, `SetupMouseSensitivity`). The
//! camera circles a point on the ground at a distance, turned about the
//! vertical and tipped from looking straight down toward the horizon.
//! Mouse and keys move the target values 30 times a second, as Unity's
//! 30 fps loop did, and the camera eases toward them.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

/// The measurements below take the terrain as 20 Unity units across;
/// Nerthus's spans one.
const UNITY_UNIT: f32 = 1.0 / 20.0;
/// Unity ran the viewport at 30 frames a second
/// (`Application.targetFrameRate`) and `StrategyCam` steps by a fixed
/// `time` of 0.06 each frame.
const STEP_SECONDS: f32 = 1.0 / 30.0;
const STEP_TIME: f32 = 0.06;
/// The Main Camera's `StrategyCam` as the scene sets it.
const INCLINE_SPEED: f32 = 2.5;
const KEY_ZOOM_SPEED_FACTOR: f32 = 0.5;
const SCROLL_SPEED: f32 = 0.06;
const MIN_ZOOM: f32 = 0.4;
const MAX_ZOOM: f32 = 250.0;
/// The scene's 0, raised by `Start`.
const MIN_INCLINATION: f32 = 0.001;
const MAX_INCLINATION: f32 = 1.0;
const MIN_ROTATION: f32 = -1.0;
const MAX_ROTATION: f32 = 1.0;
const LOOK_AT_MIN: Vec3 = Vec3::new(-100.0, 0.0, -100.0);
const LOOK_AT_MAX: Vec3 = Vec3::new(300.0, 20.0, 300.0);
/// `rotateAndInclineSmooth` and `scrollAndZoomSmooth`: each frame the
/// camera goes `time / smooth` (0.6) of the way to its target.
const SMOOTHING: f32 = 0.1;
/// Unity's mouse axes are the movement times 0.1 (`InputManager`).
const MOUSE_AXIS: f32 = 0.1;
/// Trackpads scroll in pixels; this many make one wheel notch.
const PIXELS_PER_NOTCH: f32 = 40.0;

/// Where the camera is (`StrategyCam.GetValues`), in Unity's units and
/// axes: X east, Y up, Z north, the terrain spanning −10 to 10. Rotation
/// is in turns clockwise from north; inclination runs from looking straight
/// down (0) to looking at the horizon (1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraValues {
    pub look_at: Vec3,
    pub inclination: f32,
    pub zoom: f32,
    pub rotation: f32,
}

impl CameraValues {
    /// The scene's starting view, which `DefaultCamera` returns to.
    pub const DEFAULT: Self = Self {
        look_at: Vec3::ZERO,
        inclination: 0.3,
        zoom: 50.0,
        rotation: 0.0,
    };

    /// From the camera's direction as the camera sees it: down for no
    /// inclination, toward the heading for full.
    fn view_direction(&self) -> Vec3 {
        let angle = self.rotation * std::f32::consts::TAU;
        let heading = Vec3::new(angle.sin(), 0.0, angle.cos());
        let down = Vec3::NEG_Y * (1.0 - self.inclination);
        (down + heading * self.inclination).normalize()
    }

    /// The camera's transform in Nerthus's world.
    pub fn transform(&self) -> Transform {
        let position = self.look_at - self.zoom * self.view_direction();
        let eye = unity_to_world(position);
        let target = unity_to_world(self.look_at);
        Transform::from_translation(eye).looking_at(target, Vec3::Y)
    }
}

/// A point in Unity's space as a point in Nerthus's (north is −Z here).
fn unity_to_world(point: Vec3) -> Vec3 {
    Vec3::new(point.x, point.y, -point.z) * UNITY_UNIT
}

/// How the mouse and keys drive the camera: the options the editor sends
/// with `SetupMouseInput` and `SetupMouseSensitivity`.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct NavigationSettings {
    /// `InvertY`: moving the mouse up tips the camera toward the horizon.
    pub invert_y: bool,
    pub invert_zoom: bool,
    /// Whether rotating, panning and dragging to zoom need Alt held (the
    /// others then need it let go).
    pub alt_rotate: bool,
    pub alt_pan: bool,
    pub alt_zoom: bool,
    pub rotate_button: MouseButton,
    pub pan_button: MouseButton,
    pub zoom_button: MouseButton,
    /// `FlipQE`: E moves up and Q down.
    pub flip_qe: bool,
    pub rotate_sensitivity: f32,
    pub pan_sensitivity: f32,
    pub zoom_sensitivity: f32,
    pub incline_sensitivity: f32,
    pub key_rotate_sensitivity: f32,
}

impl Default for NavigationSettings {
    /// The default options: the left button rotates, the middle pans, the
    /// right zooms.
    fn default() -> Self {
        Self {
            invert_y: true,
            invert_zoom: false,
            alt_rotate: false,
            alt_pan: false,
            alt_zoom: false,
            rotate_button: MouseButton::Left,
            pan_button: MouseButton::Middle,
            zoom_button: MouseButton::Right,
            flip_qe: false,
            rotate_sensitivity: 1.0,
            pan_sensitivity: 1.0,
            zoom_sensitivity: 1.0,
            incline_sensitivity: 1.0,
            key_rotate_sensitivity: 1.0,
        }
    }
}

impl NavigationSettings {
    /// Unity's mouse button numbers (0 left, 1 right, 2 middle).
    pub fn button(number: i64) -> MouseButton {
        match number {
            1 => MouseButton::Right,
            2 => MouseButton::Middle,
            _ => MouseButton::Left,
        }
    }
}

/// Whether the viewport has the mouse and the keyboard, set by the
/// interface every frame: the pointer is over the 3D view and nothing else
/// wants it; the view was clicked last and no text field is being typed in.
#[derive(Resource, Default)]
pub struct ViewportInput {
    pub enabled: bool,
    pub keys: bool,
}

/// Asks the camera to jump somewhere (`SetCamera`, `ApplyValues`).
#[derive(Resource, Default)]
pub struct CameraRequest {
    pub apply: Option<CameraValues>,
}

/// The keys that move the camera, as held this step.
#[derive(Clone, Copy, Debug, Default)]
struct HeldKeys {
    right: bool,
    left: bool,
    forward: bool,
    back: bool,
    up: bool,
    down: bool,
    rotate_left: bool,
    rotate_right: bool,
    incline_up: bool,
    incline_down: bool,
    zoom_in: bool,
    zoom_out: bool,
}

/// What one step of `LateUpdate` sees.
#[derive(Clone, Copy, Debug, Default)]
struct StepInput {
    /// Mouse movement in pixels, Y up.
    motion: Vec2,
    /// `Mouse ScrollWheel`.
    scroll: f32,
    rotate_held: bool,
    pan_held: bool,
    zoom_held: bool,
    alt: bool,
    keys: HeldKeys,
    /// Home or Space: back over the middle of the terrain.
    recentre: bool,
}

/// The camera's state (`StrategyCam`).
#[derive(Component, Clone, Debug)]
pub struct StrategyCamera {
    current: CameraValues,
    target: CameraValues,
    /// `initialLookAt.y`: the height Home and Space return to; Space first
    /// sets it from the terrain (`DoLookAt`).
    centre_height: f32,
    /// Time not yet stepped, and the input gathered meanwhile.
    clock: f32,
    motion: Vec2,
    scroll: f32,
    recentre: bool,
    /// Rotating since the last step (the first step of a drag only notes
    /// where the mouse is).
    rotating: bool,
    /// Buttons pressed while over the view, until let go.
    held: [bool; 3],
    /// Panning now (Unity shows its move cursor).
    panning: bool,
}

impl Default for StrategyCamera {
    fn default() -> Self {
        Self {
            current: CameraValues::DEFAULT,
            target: CameraValues::DEFAULT,
            centre_height: 0.0,
            clock: 0.0,
            motion: Vec2::ZERO,
            scroll: 0.0,
            recentre: false,
            rotating: false,
            held: [false; 3],
            panning: false,
        }
    }
}

impl StrategyCamera {
    /// `GetValues`.
    pub fn values(&self) -> CameraValues {
        self.current
    }

    /// `ApplyValues`: there at once, with nothing left to ease toward.
    pub fn apply(&mut self, values: CameraValues) {
        self.current = values;
        self.target = values;
    }

    /// Where north (the map's top) is on screen: radians clockwise from
    /// up, as the compass turns (`CompassBehavior`).
    pub fn north_angle(&self) -> f32 {
        -self.current.rotation * std::f32::consts::TAU
    }

    pub fn transform(&self) -> Transform {
        self.current.transform()
    }

    /// Whether the pan button is held as panning needs.
    pub fn panning(&self) -> bool {
        self.panning
    }

    /// `DoLookAt`: the highest of every 64th height, ten times over.
    fn centre_on(&mut self, heights: &[f32]) {
        let highest = heights.iter().step_by(64).fold(0.0_f64, |highest, &height| highest.max(height as f64));
        self.centre_height = (highest * 10.0) as f32;
    }

    /// One frame of `StrategyCam.LateUpdate`, moving the targets.
    fn step(&mut self, input: &StepInput, settings: &NavigationSettings) {
        let rotate_speed = 2.0 * settings.rotate_sensitivity;
        let pan_speed = 0.015 * settings.pan_sensitivity;
        let zoom_speed = 3.0 * settings.zoom_sensitivity;
        let key_incline_speed_factor = 0.025 * settings.incline_sensitivity;
        let key_rotate_speed_factor = 0.005 * settings.key_rotate_sensitivity;

        // The camera's right and its up laid flat, from where it stands.
        let angle = self.current.rotation * std::f32::consts::TAU;
        let x_direction = Vec3::new(angle.cos(), 0.0, -angle.sin());
        let looks_level = self.current.inclination >= MAX_INCLINATION;
        let y_direction = if looks_level { Vec3::ZERO } else { Vec3::new(angle.sin(), 0.0, angle.cos()) };
        let pan_horizontal = input.motion.x * MOUSE_AXIS;
        let pan_vertical = input.motion.y * MOUSE_AXIS;

        if input.recentre {
            self.target.look_at = Vec3::new(0.0, self.centre_height, 0.0);
        }
        let alt_matches = |needs_alt: bool| needs_alt == input.alt;
        let target_zoom = self.target.zoom;
        if input.pan_held && alt_matches(settings.alt_pan) {
            let pan_step = pan_speed * target_zoom;
            if pan_horizontal < 0.0 {
                self.target.look_at += x_direction * pan_step;
            }
            if pan_horizontal > 0.0 {
                self.target.look_at -= x_direction * pan_step;
            }
            if pan_vertical < 0.0 {
                self.target.look_at += y_direction * pan_step;
            }
            if pan_vertical > 0.0 {
                self.target.look_at -= y_direction * pan_step;
            }
        }

        let key_step = SCROLL_SPEED * STEP_TIME * target_zoom;
        let keys = input.keys;
        if keys.up {
            self.target.look_at.y += key_step;
        }
        if keys.down {
            self.target.look_at.y -= key_step;
        }
        if keys.right {
            self.target.look_at += x_direction * key_step;
        }
        if keys.left {
            self.target.look_at -= x_direction * key_step;
        }
        if keys.forward {
            self.target.look_at += y_direction * key_step;
        }
        if keys.back {
            self.target.look_at -= y_direction * key_step;
        }

        let mut zoom_request = input.scroll;
        let drags_zoom = input.zoom_held && !input.pan_held && !input.rotate_held && alt_matches(settings.alt_zoom);
        if zoom_request == 0.0 && drags_zoom {
            let direction = if settings.invert_zoom { -1.0 } else { 1.0 };
            zoom_request = direction * pan_vertical * STEP_TIME;
        }
        if keys.zoom_in {
            zoom_request += KEY_ZOOM_SPEED_FACTOR * STEP_TIME;
        }
        if keys.zoom_out {
            zoom_request -= KEY_ZOOM_SPEED_FACTOR * STEP_TIME;
        }
        if zoom_request != 0.0 {
            let zoomed = self.target.zoom - zoom_speed * self.target.zoom * zoom_request;
            self.target.zoom = zoomed.clamp(MIN_ZOOM, MAX_ZOOM);
        }

        let mut rotate_request = 0.0;
        let mut incline_request = 0.0;
        let alt_allows_rotate = !settings.alt_rotate || input.alt;
        let to_rotate = alt_allows_rotate && input.rotate_held && !input.zoom_held && !input.pan_held;
        if to_rotate {
            if self.rotating {
                rotate_request += rotate_speed * 0.001 * input.motion.x;
                let incline = INCLINE_SPEED * 0.001 * input.motion.y;
                incline_request += if settings.invert_y { incline } else { -incline };
            } else {
                self.rotating = true;
            }
        } else {
            self.rotating = false;
        }
        if keys.rotate_left {
            rotate_request += key_rotate_speed_factor * rotate_speed * STEP_TIME;
        }
        if keys.rotate_right {
            rotate_request -= key_rotate_speed_factor * rotate_speed * STEP_TIME;
        }
        if keys.incline_up {
            incline_request += key_incline_speed_factor * INCLINE_SPEED * STEP_TIME;
        }
        if keys.incline_down {
            incline_request -= key_incline_speed_factor * INCLINE_SPEED * STEP_TIME;
        }
        if rotate_request != 0.0 {
            let rotated = self.target.rotation + rotate_request;
            self.target.rotation = rotated.clamp(MIN_ROTATION, MAX_ROTATION);
            // Kept within half a turn either way, the camera with it.
            if self.target.rotation > 0.5 {
                self.target.rotation -= 1.0;
                self.current.rotation -= 1.0;
            } else if self.target.rotation < -0.5 {
                self.target.rotation += 1.0;
                self.current.rotation += 1.0;
            }
        }
        if incline_request != 0.0 {
            let inclined = self.target.inclination + incline_request;
            self.target.inclination = inclined.clamp(MIN_INCLINATION, MAX_INCLINATION);
        }

        self.target.look_at = self.target.look_at.clamp(LOOK_AT_MIN, LOOK_AT_MAX);
    }

    /// Eases the camera toward its targets over `seconds`: 0.6 of the way
    /// each 30th of a second.
    fn ease(&mut self, seconds: f32) {
        let frames = seconds / STEP_SECONDS;
        let remaining = (1.0 - STEP_TIME / SMOOTHING).powf(frames);
        let current = &mut self.current;
        let target = &self.target;
        current.look_at = target.look_at + (current.look_at - target.look_at) * remaining;
        current.zoom = target.zoom + (current.zoom - target.zoom) * remaining;
        current.rotation = target.rotation + (current.rotation - target.rotation) * remaining;
        current.inclination = target.inclination + (current.inclination - target.inclination) * remaining;
    }
}

/// The buttons in `held`'s order.
const BUTTONS: [MouseButton; 3] = [MouseButton::Left, MouseButton::Right, MouseButton::Middle];

/// Reads the mouse and keys, steps the camera 30 times a second and eases
/// it toward where the steps leave it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn strategy_camera(
    input: Res<ViewportInput>,
    settings: Res<NavigationSettings>,
    mut request: ResMut<CameraRequest>,
    preview: Res<crate::TerrainPreview>,
    buttons: Res<ButtonInput<MouseButton>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time<Real>>,
    handle: Res<crate::gizmo3d::PositionHandle>,
    camera: Single<(&mut StrategyCamera, &mut Transform)>,
) {
    let (mut camera, mut transform) = camera.into_inner();
    if let Some(values) = request.apply.take() {
        camera.apply(values);
    }

    // Unity only sees a button pressed over its window, until let go.
    // A gizmo handle under the pointer takes the left button
    // (`StartGizmoMovements`).
    let taken = |button: MouseButton| button == MouseButton::Left && handle.holds_pointer();
    for (index, button) in BUTTONS.iter().enumerate() {
        if buttons.just_pressed(*button) && input.enabled && !taken(*button) {
            camera.held[index] = true;
        }
        if !buttons.pressed(*button) {
            camera.held[index] = false;
        }
    }
    let dragging = camera.held.iter().any(|held| *held);
    if input.enabled || dragging {
        camera.motion += Vec2::new(motion.delta.x, -motion.delta.y);
    }
    if input.enabled {
        let notches = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / PIXELS_PER_NOTCH,
        };
        camera.scroll += notches * MOUSE_AXIS;
    }
    let recentre_pressed = keyboard.just_pressed(KeyCode::Home) || keyboard.just_pressed(KeyCode::Space);
    if input.keys && recentre_pressed {
        if keyboard.just_pressed(KeyCode::Space)
            && let Some(heights) = &preview.heights
        {
            camera.centre_on(heights.data());
        }
        camera.recentre = true;
    }

    let held_button = |button: MouseButton| BUTTONS.iter().position(|candidate| *candidate == button).is_some_and(|index| camera.held[index]);
    let key = |code: KeyCode| input.keys && keyboard.pressed(code);
    let (up_key, down_key) = if settings.flip_qe { (KeyCode::KeyE, KeyCode::KeyQ) } else { (KeyCode::KeyQ, KeyCode::KeyE) };
    let held_keys = HeldKeys {
        right: key(KeyCode::KeyD) || key(KeyCode::ArrowRight),
        left: key(KeyCode::KeyA) || key(KeyCode::ArrowLeft),
        forward: key(KeyCode::KeyW) || key(KeyCode::ArrowUp),
        back: key(KeyCode::KeyS) || key(KeyCode::ArrowDown),
        up: key(up_key),
        down: key(down_key),
        rotate_left: key(KeyCode::KeyH),
        rotate_right: key(KeyCode::KeyK),
        incline_up: key(KeyCode::KeyJ),
        incline_down: key(KeyCode::KeyU),
        zoom_in: key(KeyCode::KeyO),
        zoom_out: key(KeyCode::KeyL),
    };
    let mut step_input = StepInput {
        motion: Vec2::ZERO,
        scroll: 0.0,
        rotate_held: held_button(settings.rotate_button),
        pan_held: held_button(settings.pan_button),
        zoom_held: held_button(settings.zoom_button),
        alt: keyboard.pressed(KeyCode::AltLeft) || keyboard.pressed(KeyCode::AltRight),
        keys: held_keys,
        recentre: false,
    };

    camera.panning = step_input.pan_held && settings.alt_pan == step_input.alt;

    let seconds = time.delta_secs();
    camera.clock += seconds;
    while camera.clock >= STEP_SECONDS {
        camera.clock -= STEP_SECONDS;
        // What came in since the last step goes to this one.
        step_input.motion = std::mem::take(&mut camera.motion);
        step_input.scroll = std::mem::take(&mut camera.scroll);
        step_input.recentre = std::mem::take(&mut camera.recentre);
        camera.step(&step_input, &settings);
    }
    camera.ease(seconds);
    *transform = camera.transform();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step_with(camera: &mut StrategyCamera, input: StepInput) {
        let settings = NavigationSettings::default();
        camera.step(&input, &settings);
    }

    #[test]
    fn default_view_looks_north_and_down() {
        let transform = CameraValues::DEFAULT.transform();
        let forward = transform.forward();
        // Down at atan(0.7 / 0.3) below the horizon, toward −Z (north).
        assert!(forward.z < 0.0 && forward.x.abs() < 1e-6);
        let below = (-forward.y).atan2(-forward.z).to_degrees();
        assert!((below - 66.80141).abs() < 1e-3);
        assert!((transform.translation.length() - 2.5).abs() < 1e-5);
    }

    #[test]
    fn wheel_notch_zooms_by_thirty_percent() {
        let mut camera = StrategyCamera::default();
        let input = StepInput {
            scroll: MOUSE_AXIS,
            ..StepInput::default()
        };
        step_with(&mut camera, input);
        assert!((camera.target.zoom - 35.0).abs() < 1e-4);
    }

    #[test]
    fn dragging_rotates_after_the_first_step() {
        let mut camera = StrategyCamera::default();
        let input = StepInput {
            motion: Vec2::new(100.0, 40.0),
            rotate_held: true,
            ..StepInput::default()
        };
        step_with(&mut camera, input);
        assert_eq!(camera.target.rotation, 0.0);
        step_with(&mut camera, input);
        assert!((camera.target.rotation - 0.2).abs() < 1e-6);
        // InvertY: moving up tips the camera toward the horizon.
        assert!((camera.target.inclination - 0.4).abs() < 1e-6);
    }

    #[test]
    fn rotation_wraps_within_half_a_turn() {
        let mut camera = StrategyCamera::default();
        camera.apply(CameraValues {
            rotation: 0.45,
            ..CameraValues::DEFAULT
        });
        camera.rotating = true;
        let input = StepInput {
            motion: Vec2::new(50.0, 0.0),
            rotate_held: true,
            ..StepInput::default()
        };
        step_with(&mut camera, input);
        assert!((camera.target.rotation + 0.45).abs() < 1e-6);
        assert!((camera.current.rotation + 0.55).abs() < 1e-6);
    }

    #[test]
    fn panning_moves_a_step_per_frame_against_the_mouse() {
        let mut camera = StrategyCamera::default();
        let input = StepInput {
            motion: Vec2::new(-3.0, 0.0),
            pan_held: true,
            ..StepInput::default()
        };
        step_with(&mut camera, input);
        // Mouse left: the point looked at moves east by 0.015 × zoom.
        assert!((camera.target.look_at.x - 0.75).abs() < 1e-6);
    }

    #[test]
    fn easing_goes_six_tenths_a_frame() {
        let mut camera = StrategyCamera::default();
        camera.target.zoom = 60.0;
        camera.ease(STEP_SECONDS);
        assert!((camera.current.zoom - 56.0).abs() < 1e-4);
    }

    /// The system in an app: a wheel notch over the view zooms in; the
    /// same over the rest of the window does nothing.
    #[test]
    fn wheel_zooms_only_over_the_view() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let frame = std::time::Duration::from_secs_f32(STEP_SECONDS);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(frame));
        app.init_resource::<ViewportInput>();
        app.init_resource::<NavigationSettings>();
        app.init_resource::<CameraRequest>();
        app.init_resource::<crate::TerrainPreview>();
        app.init_resource::<crate::gizmo3d::PositionHandle>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<AccumulatedMouseMotion>();
        let notch = AccumulatedMouseScroll {
            unit: MouseScrollUnit::Line,
            delta: Vec2::new(0.0, 1.0),
        };
        app.insert_resource(notch);
        app.add_systems(Update, strategy_camera);
        let camera = app.world_mut().spawn((StrategyCamera::default(), Transform::default())).id();

        for _ in 0..3 {
            app.update();
        }
        let untouched = app.world().get::<StrategyCamera>(camera).map(|camera| camera.target.zoom);
        assert_eq!(untouched, Some(50.0));

        app.world_mut().resource_mut::<ViewportInput>().enabled = true;
        for _ in 0..3 {
            app.update();
        }
        let zoomed = app.world().get::<StrategyCamera>(camera).map(|camera| camera.target.zoom).unwrap_or_default();
        assert!(zoomed < 50.0, "zoom stayed at {zoomed}");
    }

    #[test]
    fn space_centres_on_the_terrain_top() {
        let mut camera = StrategyCamera::default();
        let mut heights = vec![0.1; 256];
        heights[64] = 0.8;
        heights[65] = 0.9;
        camera.centre_on(&heights);
        assert!((camera.centre_height - 8.0).abs() < 1e-5);
    }
}

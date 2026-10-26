//! Property motion independent of GPUI's bounded animation progress. A spring
//! may overshoot its target; its values never pass through Animation's easing
//! channel. Owners retain these states by stable item identity and request
//! frames only while a returned sample is active.

use std::time::Instant;

use super::{ATTACHMENT_OUT, CHIP_IN, MotionSpec, lerp, speed_scale};

/// Physical parameters in property units and seconds. Thresholds belong to the
/// property (a scale needs tighter thresholds than a pixel translation).
#[derive(Clone, Copy, Debug)]
pub struct SpringSpec {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
    pub position_epsilon: f32,
    pub velocity_epsilon: f32,
}

impl SpringSpec {
    pub const fn new(stiffness: f32, damping: f32, mass: f32) -> Self {
        Self {
            stiffness,
            damping,
            mass,
            position_epsilon: 0.001,
            velocity_epsilon: 0.01,
        }
    }

    pub const fn with_thresholds(mut self, position: f32, velocity: f32) -> Self {
        self.position_epsilon = position;
        self.velocity_epsilon = velocity;
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SpringSample {
    pub value: f32,
    /// Property units per simulated second, preserved when retargeting.
    pub velocity: f32,
    pub active: bool,
}

#[derive(Clone, Debug)]
pub struct PropertySpring {
    spec: SpringSpec,
    from: f32,
    target: f32,
    velocity: f32,
    started: Instant,
}

impl PropertySpring {
    pub fn new(value: f32, spec: SpringSpec, now: Instant) -> Self {
        let value = if value.is_finite() { value } else { 0.0 };
        Self {
            spec,
            from: value,
            target: value,
            velocity: 0.0,
            started: now,
        }
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn snap(&mut self, value: f32, now: Instant) {
        if value.is_finite() {
            self.from = value;
            self.target = value;
            self.velocity = 0.0;
            self.started = now;
        }
    }

    /// Repeated renders of the same target do not restart the clock. A changed
    /// target continues with the sampled position AND velocity.
    pub fn retarget(&mut self, target: f32, reduced: bool, now: Instant) {
        if !target.is_finite() {
            return;
        }
        if reduced {
            self.snap(target, now);
        } else if target != self.target {
            let current = self.sample(now, false);
            self.from = current.value;
            self.velocity = current.velocity;
            self.target = target;
            self.started = now;
        }
    }

    /// A drag release can supply the actual pointer velocity rather than
    /// replacing it with a guessed fixed-duration curve.
    pub fn retarget_with_velocity(
        &mut self,
        target: f32,
        velocity: f32,
        reduced: bool,
        now: Instant,
    ) {
        if !target.is_finite() || !velocity.is_finite() {
            return;
        }
        let current = self.sample(now, reduced);
        self.from = current.value;
        self.target = target;
        self.velocity = if reduced { 0.0 } else { velocity };
        self.started = now;
        if reduced {
            self.snap(target, now);
        }
    }

    pub fn sample(&self, now: Instant, reduced: bool) -> SpringSample {
        let rest = SpringSample {
            value: self.target,
            velocity: 0.0,
            active: false,
        };
        if reduced || (self.from == self.target && self.velocity == 0.0) {
            return rest;
        }
        let positive = |value: f32, fallback: f32| {
            if value.is_finite() && value > 0.0 {
                value.clamp(0.0001, 1_000_000.0)
            } else {
                fallback
            }
        };
        let k = positive(self.spec.stiffness, 600.0);
        let m = positive(self.spec.mass, 1.0);
        let c = positive(self.spec.damping, 30.0);
        let t = now.saturating_duration_since(self.started).as_secs_f32() / speed_scale();
        // Malformed/nearly undamped specs must not retain an idle frame loop.
        if t >= 8.0 {
            return rest;
        }
        let alpha = c / (2.0 * m);
        let omega_sq = k / m;
        let discriminant = alpha * alpha - omega_sq;
        let displacement = self.from - self.target;
        let (position, velocity) = if discriminant.abs() <= omega_sq * 0.0001 {
            let b = self.velocity + alpha * displacement;
            let position = displacement + b * t;
            let decay = (-alpha * t).exp();
            (position * decay, (b - alpha * position) * decay)
        } else if discriminant < 0.0 {
            let omega = (-discriminant).sqrt();
            let b = (self.velocity + alpha * displacement) / omega;
            let (sin, cos) = (omega * t).sin_cos();
            let position = displacement * cos + b * sin;
            let derivative = omega * (-displacement * sin + b * cos);
            let decay = (-alpha * t).exp();
            (position * decay, (derivative - alpha * position) * decay)
        } else {
            let root = discriminant.sqrt();
            // Avoid cancellation in -alpha + root for a very overdamped spring.
            let slow = -omega_sq / (alpha + root);
            let fast = -(alpha + root);
            let a = (self.velocity - fast * displacement) / (slow - fast);
            let b = displacement - a;
            let slow_part = a * (slow * t).exp();
            let fast_part = b * (fast * t).exp();
            (slow_part + fast_part, slow * slow_part + fast * fast_part)
        };
        if !position.is_finite()
            || !velocity.is_finite()
            || (position.abs() <= positive(self.spec.position_epsilon, 0.001)
                && velocity.abs() <= positive(self.spec.velocity_epsilon, 0.01))
        {
            return rest;
        }
        SpringSample {
            value: self.target + position,
            velocity,
            active: true,
        }
    }
}

/// The bounded paint properties of a chip/attachment lifecycle. `scale` and
/// `blur` are for custom-painted primitives only; a normal div should consume
/// opacity/offset_y, not emulate whole-subtree scaling by changing its layout.
#[derive(Clone, Copy, Debug)]
pub struct PresenceSample {
    pub opacity: f32,
    pub offset_y: f32,
    pub scale: f32,
    pub blur: f32,
    pub active: bool,
    /// Only true after exit; the owner may then release the retained item.
    pub finished: bool,
}

impl PresenceSample {
    const PRESENT: Self = Self {
        opacity: 1.0,
        offset_y: 0.0,
        scale: 1.0,
        blur: 0.0,
        active: false,
        finished: false,
    };

    const ENTER: Self = Self {
        opacity: 0.0,
        offset_y: 4.0,
        scale: 0.8,
        blur: 2.0,
        ..Self::PRESENT
    };

    const EXIT: Self = Self {
        opacity: 0.0,
        offset_y: -2.0,
        scale: 0.5,
        blur: 0.0,
        finished: true,
        ..Self::PRESENT
    };
}

/// Keep this beside the item's stable business ID, not the transient element
/// ID of a virtualized render. Sampling is pure, so remounts never replay it.
#[derive(Clone, Debug)]
pub struct Presence {
    from: PresenceSample,
    started: Instant,
    spec: MotionSpec,
    exiting: bool,
}

impl Presence {
    pub fn enter(index: usize, now: Instant, reduced: bool) -> Self {
        Self {
            from: if reduced {
                PresenceSample::PRESENT
            } else {
                PresenceSample::ENTER
            },
            started: now,
            spec: if reduced {
                CHIP_IN.with_delay(0)
            } else {
                CHIP_IN.with_delay(50 + index.min(8) as u64 * 30)
            },
            exiting: false,
        }
    }

    /// A restored/history item has no fresh entrance animation.
    pub fn present(now: Instant) -> Self {
        Self {
            from: PresenceSample::PRESENT,
            started: now,
            spec: CHIP_IN,
            exiting: false,
        }
    }

    pub fn duration(&self) -> std::time::Duration {
        self.spec.total().mul_f32(speed_scale())
    }

    pub fn exit(&mut self, now: Instant, reduced: bool) {
        if self.exiting {
            return;
        }
        self.from = self.sample(now, reduced);
        self.started = now;
        self.spec = ATTACHMENT_OUT;
        self.exiting = true;
        if reduced || self.from.opacity == 0.0 {
            self.from = PresenceSample::EXIT;
        }
    }

    /// Undo/reappearance continues from the visible exit sample, without a
    /// fresh stagger delay or a flash back to the original entrance state.
    pub fn revive(&mut self, now: Instant, reduced: bool) {
        if !self.exiting {
            return;
        }
        self.from = self.sample(now, reduced);
        self.started = now;
        self.spec = CHIP_IN;
        self.exiting = false;
        self.from.finished = false;
        if reduced {
            self.from = PresenceSample::PRESENT;
        }
    }

    pub fn sample(&self, now: Instant, reduced: bool) -> PresenceSample {
        let end = if self.exiting {
            PresenceSample::EXIT
        } else {
            PresenceSample::PRESENT
        };
        let settled = self.from.opacity == end.opacity
            && self.from.offset_y == end.offset_y
            && self.from.scale == end.scale
            && self.from.blur == end.blur;
        if reduced || settled {
            return end;
        }
        let raw = now.saturating_duration_since(self.started).as_secs_f32()
            / self.duration().as_secs_f32().max(f32::EPSILON);
        if raw >= 1.0 {
            return end;
        }
        let t = self.spec.progress(raw);
        PresenceSample {
            opacity: lerp(self.from.opacity, end.opacity, t).clamp(0.0, 1.0),
            offset_y: lerp(self.from.offset_y, end.offset_y, t),
            scale: lerp(self.from.scale, end.scale, t),
            blur: lerp(self.from.blur, end.blur, t),
            active: true,
            finished: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn later(now: Instant, millis: u64) -> Instant {
        now + Duration::from_millis(millis).mul_f32(speed_scale())
    }

    #[test]
    fn property_can_overshoot_without_unbounded_animation_progress() {
        let now = Instant::now();
        let mut spring = PropertySpring::new(0.0, SpringSpec::new(260.0, 16.0, 1.0), now);
        spring.retarget(1.0, false, now);
        assert!((0..80).any(|i| spring.sample(later(now, i * 10), false).value > 1.01));
        let end = spring.sample(later(now, 3000), false);
        assert!(!end.active);
        assert_eq!(end.value, 1.0);
    }

    #[test]
    fn reversal_preserves_position_and_velocity_and_same_target_does_not_restart() {
        let now = Instant::now();
        let mut spring = PropertySpring::new(0.0, SpringSpec::new(600.0, 30.0, 1.0), now);
        spring.retarget(1.0, false, now);
        let turn = later(now, 50);
        let before = spring.sample(turn, false);
        spring.retarget(0.0, false, turn);
        let after = spring.sample(turn, false);
        assert!((before.value - after.value).abs() < 0.00001);
        assert!((before.velocity - after.velocity).abs() < 0.0001);
        let later = later(turn, 40);
        let expected = spring.sample(later, false);
        spring.retarget(0.0, false, later);
        let unchanged = spring.sample(later, false);
        assert_eq!(expected.value, unchanged.value);
        assert_eq!(expected.velocity, unchanged.velocity);
    }

    #[test]
    fn drag_velocity_and_reduced_motion_are_explicit() {
        let now = Instant::now();
        let mut spring = PropertySpring::new(0.0, SpringSpec::new(320.0, 28.0, 1.0), now);
        spring.retarget_with_velocity(0.0, 20.0, false, now);
        assert_eq!(spring.sample(now, false).velocity, 20.0);
        assert!(spring.sample(later(now, 20), false).value > 0.0);
        spring.retarget(2.0, true, later(now, 30));
        let sample = spring.sample(later(now, 31), false);
        assert_eq!(sample.value, 2.0);
        assert_eq!(sample.velocity, 0.0);
        assert!(!sample.active);
    }

    #[test]
    fn all_damping_regimes_settle_and_invalid_inputs_remain_finite() {
        let now = Instant::now();
        for spec in [
            SpringSpec::new(100.0, 10.0, 1.0),
            SpringSpec::new(100.0, 20.0, 1.0),
            SpringSpec::new(100.0, 30.0, 1.0),
            SpringSpec::new(f32::NAN, -1.0, 0.0),
        ] {
            let mut spring = PropertySpring::new(0.0, spec, now);
            spring.retarget(1.0, false, now);
            for i in 0..100 {
                let sample = spring.sample(later(now, i * 40), false);
                assert!(sample.value.is_finite() && sample.velocity.is_finite());
            }
            assert!(!spring.sample(later(now, 8000), false).active);
        }
    }

    #[test]
    fn presence_stagger_is_capped_and_stable_sampling_does_not_replay() {
        let now = Instant::now();
        let last = Presence::enter(8, now, false);
        let extra = Presence::enter(200, now, false);
        assert_eq!(last.duration(), extra.duration());
        assert_eq!(
            last.duration(),
            Duration::from_millis(490).mul_f32(speed_scale())
        );
        assert_eq!(last.sample(later(now, 289), false).opacity, 0.0);
        let end = last.sample(later(now, 600), false);
        assert_eq!(end.opacity, 1.0);
        assert!(!end.active && !end.finished);
        assert_eq!(last.sample(later(now, 700), false).opacity, 1.0);
    }

    #[test]
    fn interrupted_exit_and_undo_continue_without_flashing() {
        let now = Instant::now();
        let mut item = Presence::enter(0, now, false);
        let exit_at = later(now, 110);
        let visible = item.sample(exit_at, false);
        item.exit(exit_at, false);
        assert_eq!(item.sample(exit_at, false).opacity, visible.opacity);
        let undo_at = later(exit_at, 60);
        let fading = item.sample(undo_at, false);
        item.revive(undo_at, false);
        assert_eq!(item.sample(undo_at, false).opacity, fading.opacity);
        assert_eq!(item.sample(later(undo_at, 300), false).opacity, 1.0);
        item.exit(later(undo_at, 300), false);
        assert!(item.sample(later(undo_at, 431), false).finished);
        item.revive(later(undo_at, 432), true);
        assert!(!item.sample(later(undo_at, 432), true).active);
    }

    #[test]
    fn reduced_exit_and_removed_before_entrance_release_immediately() {
        let now = Instant::now();
        let mut item = Presence::enter(3, now, false);
        item.exit(now, false);
        assert!(item.sample(now, false).finished);
        let mut item = Presence::present(now);
        item.exit(now, true);
        assert!(item.sample(now, false).finished);
    }
}

use std::{
    sync::{
        Condvar, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct FrameWake {
    gate: Mutex<()>,
    ready: Condvar,
}

static FRAME_WAKE: LazyLock<FrameWake> = LazyLock::new(FrameWake::default);

impl FrameWake {
    fn wake(&self) {
        let _guard = self.gate.lock().unwrap_or_else(|error| error.into_inner());
        self.ready.notify_all();
    }

    fn wait_while_idle(&self, mut idle: impl FnMut() -> bool) -> bool {
        if !idle() {
            return false;
        }
        let guard = self.gate.lock().unwrap_or_else(|error| error.into_inner());
        let result = self
            .ready
            .wait_timeout_while(guard, Duration::from_secs(1), |_| idle())
            .unwrap_or_else(|error| error.into_inner());
        result.1.timed_out()
    }
}

pub(crate) fn wake_frame_loop() {
    FRAME_WAKE.wake();
}

pub(crate) fn wait_while_idle(idle: impl FnMut() -> bool) {
    FRAME_WAKE.wait_while_idle(idle);
}

pub(crate) struct FrameDemand {
    requested: AtomicBool,
    suspended: AtomicBool,
}

impl Default for FrameDemand {
    fn default() -> Self {
        Self {
            requested: AtomicBool::new(true),
            suspended: AtomicBool::new(false),
        }
    }
}

impl FrameDemand {
    pub(crate) fn request(&self) {
        if !self.requested.swap(true, Ordering::AcqRel) && !self.suspended.load(Ordering::Acquire) {
            wake_frame_loop();
        }
    }

    pub(crate) fn pause(&self) {
        self.requested.store(false, Ordering::Release);
    }

    pub(crate) fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Acquire) && !self.suspended.load(Ordering::Acquire)
    }

    pub(crate) fn suspend(&self) {
        self.suspended.store(true, Ordering::Release);
    }

    pub(crate) fn resume(&self) {
        self.suspended.store(false, Ordering::Release);
        self.requested.store(true, Ordering::Release);
        wake_frame_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_and_animation_continue_until_idle() {
        let demand = FrameDemand::default();
        assert!(demand.is_requested());
        assert!(demand.is_requested());
        demand.pause();
        assert!(!demand.is_requested());
        demand.request();
        assert!(demand.is_requested());
    }

    #[test]
    fn windows_have_independent_frame_demand() {
        let idle = FrameDemand::default();
        let animated = FrameDemand::default();
        idle.pause();
        assert!(!idle.is_requested());
        assert!(animated.is_requested());
    }

    #[test]
    fn frame_requested_during_idle_transition_is_not_lost() {
        let wake = std::sync::Arc::new(FrameWake::default());
        let idle = std::sync::Arc::new(AtomicBool::new(true));
        let (observed_tx, observed_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let worker_wake = wake.clone();
        let worker_idle = idle.clone();
        let worker = std::thread::spawn(move || {
            let timed_out = worker_wake.wait_while_idle(|| {
                let waiting = worker_idle.load(Ordering::Acquire);
                let _ = observed_tx.send(());
                waiting
            });
            finished_tx
                .send((worker_idle.load(Ordering::Acquire), timed_out))
                .unwrap();
        });
        observed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        idle.store(false, Ordering::Release);
        wake.wake();
        assert_eq!(
            finished_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            (false, false)
        );
        worker.join().unwrap();
    }

    #[test]
    fn active_frames_skip_the_idle_wait() {
        let wake = FrameWake::default();
        let mut checks = 0;
        wake.wait_while_idle(|| {
            checks += 1;
            false
        });
        assert_eq!(checks, 1);
    }

    #[test]
    fn idle_wait_retains_a_device_health_watchdog() {
        assert!(FrameWake::default().wait_while_idle(|| true));
    }

    #[test]
    fn minimized_updates_wait_for_restore() {
        let demand = FrameDemand::default();
        demand.suspend();
        demand.request();
        assert!(!demand.is_requested());
        demand.resume();
        assert!(demand.is_requested());
    }

    #[test]
    fn repeated_minimize_and_restore_preserve_frame_demand() {
        let demand = FrameDemand::default();
        demand.pause();
        demand.suspend();
        demand.suspend();
        demand.request();
        demand.request();
        assert!(!demand.is_requested());
        demand.resume();
        assert!(demand.is_requested());
        demand.pause();
        assert!(!demand.is_requested());
    }
}

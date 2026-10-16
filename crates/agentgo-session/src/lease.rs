//! Tab and input leases.
//!
//! Leases serialize who owns a tab's input at a given time so two attached
//! agents (or a workflow and the human) cannot cross-control. Time is
//! injected via [`Clock`] so expiry is fully deterministic in tests.

use std::time::{Duration, Instant};

use crate::SessionError;

/// Monotonic clock abstraction (deterministic in tests).
pub trait Clock: std::fmt::Debug {
    /// Current monotonic reading.
    fn now(&self) -> Instant;
}

/// Real system clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// What a lease covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeaseKind {
    /// Exclusive input dispatch on a tab.
    TabInput,
    /// Exclusive ownership of a profile for a workflow.
    Profile,
}

/// An active lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// Lease key, e.g. `tab:t-1` or `profile:p-2`.
    pub key: String,
    /// Owning client id.
    pub holder: String,
    /// Lease kind.
    pub kind: LeaseKind,
    /// Absolute expiry (monotonic).
    pub expires_at: Instant,
}

/// In-memory lease registry (per instance).
#[derive(Debug)]
pub struct LeaseRegistry<C: Clock = SystemClock> {
    clock: C,
    leases: Vec<Lease>,
}

impl Default for LeaseRegistry<SystemClock> {
    fn default() -> Self {
        Self::new(SystemClock)
    }
}

impl<C: Clock> LeaseRegistry<C> {
    /// Creates a registry with the given clock.
    pub fn new(clock: C) -> Self {
        Self {
            clock,
            leases: Vec::new(),
        }
    }

    /// Tries to acquire `key` for `holder` for `ttl`. Fails if a live lease
    /// is held by someone else.
    pub fn acquire(
        &mut self,
        key: &str,
        holder: &str,
        kind: LeaseKind,
        ttl: Duration,
    ) -> Result<Lease, SessionError> {
        self.evict_expired();
        if let Some(current) = self.leases.iter().find(|l| l.key == key) {
            if current.holder != holder {
                return Err(SessionError::LeaseHeld {
                    lease: key.to_string(),
                    holder: current.holder.clone(),
                });
            }
        }
        let lease = Lease {
            key: key.to_string(),
            holder: holder.to_string(),
            kind,
            expires_at: self.clock.now() + ttl,
        };
        self.leases.retain(|l| l.key != key);
        self.leases.push(lease.clone());
        Ok(lease)
    }

    /// Renews an existing lease for `holder` by `ttl`.
    pub fn renew(&mut self, key: &str, holder: &str, ttl: Duration) -> Result<Lease, SessionError> {
        self.evict_expired();
        let Some(idx) = self
            .leases
            .iter()
            .position(|l| l.key == key && l.holder == holder)
        else {
            return Err(SessionError::LeaseNotHeld(key.to_string()));
        };
        self.leases[idx].expires_at = self.clock.now() + ttl;
        Ok(self.leases[idx].clone())
    }

    /// Releases a lease held by `holder`.
    pub fn release(&mut self, key: &str, holder: &str) -> Result<(), SessionError> {
        let before = self.leases.len();
        self.leases
            .retain(|l| !(l.key == key && l.holder == holder));
        if self.leases.len() == before {
            return Err(SessionError::LeaseNotHeld(key.to_string()));
        }
        Ok(())
    }

    /// Current live lease for a key, if any.
    pub fn current(&mut self, key: &str) -> Option<Lease> {
        self.evict_expired();
        self.leases.iter().find(|l| l.key == key).cloned()
    }

    fn evict_expired(&mut self) {
        let now = self.clock.now();
        self.leases.retain(|l| l.expires_at > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Deterministic stepping clock, cheaply cloneable so tests can keep
    /// a handle while the registry owns another.
    #[derive(Debug, Clone)]
    struct FakeClock {
        base: Instant,
        offset: std::rc::Rc<std::cell::Cell<Duration>>,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                base: Instant::now(),
                offset: std::rc::Rc::new(std::cell::Cell::new(Duration::ZERO)),
            }
        }
        fn advance(&self, d: Duration) {
            self.offset.set(self.offset.get() + d);
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + self.offset.get()
        }
    }

    #[test]
    fn exclusive_acquisition_and_conflict() {
        let mut r = LeaseRegistry::new(FakeClock::new());
        r.acquire(
            "tab:t-1",
            "client-a",
            LeaseKind::TabInput,
            Duration::from_secs(30),
        )
        .unwrap();
        match r.acquire(
            "tab:t-1",
            "client-b",
            LeaseKind::TabInput,
            Duration::from_secs(30),
        ) {
            Err(SessionError::LeaseHeld { holder, .. }) => assert_eq!(holder, "client-a"),
            other => panic!("expected LeaseHeld, got {other:?}"),
        }
        // Same holder reacquires (refresh).
        r.acquire(
            "tab:t-1",
            "client-a",
            LeaseKind::TabInput,
            Duration::from_secs(30),
        )
        .unwrap();
    }

    #[test]
    fn leases_expire_and_free_the_key() {
        let clock = FakeClock::new();
        let mut r = LeaseRegistry::new(clock.clone());
        r.acquire(
            "tab:t-1",
            "client-a",
            LeaseKind::TabInput,
            Duration::from_secs(10),
        )
        .unwrap();
        clock.advance(Duration::from_secs(11));
        assert!(r.current("tab:t-1").is_none());
        r.acquire(
            "tab:t-1",
            "client-b",
            LeaseKind::TabInput,
            Duration::from_secs(10),
        )
        .unwrap();
    }

    #[test]
    fn release_then_reacquire_by_other() {
        let mut r = LeaseRegistry::new(FakeClock::new());
        r.acquire(
            "profile:p-2",
            "client-a",
            LeaseKind::Profile,
            Duration::from_secs(60),
        )
        .unwrap();
        r.release("profile:p-2", "client-a").unwrap();
        assert!(matches!(
            r.release("profile:p-2", "client-a"),
            Err(SessionError::LeaseNotHeld(_))
        ));
        r.acquire(
            "profile:p-2",
            "client-b",
            LeaseKind::Profile,
            Duration::from_secs(60),
        )
        .unwrap();
    }

    #[test]
    fn renew_extends_life() {
        let clock = FakeClock::new();
        let mut r = LeaseRegistry::new(clock.clone());
        r.acquire(
            "tab:t-9",
            "client-a",
            LeaseKind::TabInput,
            Duration::from_secs(10),
        )
        .unwrap();
        clock.advance(Duration::from_secs(8));
        r.renew("tab:t-9", "client-a", Duration::from_secs(10))
            .unwrap();
        clock.advance(Duration::from_secs(8));
        assert!(
            r.current("tab:t-9").is_some(),
            "renewed lease must still be live"
        );
        assert!(matches!(
            r.renew("tab:t-9", "client-b", Duration::from_secs(10)),
            Err(SessionError::LeaseNotHeld(_))
        ));
    }
}

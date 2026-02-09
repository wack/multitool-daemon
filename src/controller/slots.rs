use super::crd::{HelmReleaseStatus, Slot, SlotPair, SlotState};

// ---------------------------------------------------------------------------
// Slot management — pure logic, no I/O
// ---------------------------------------------------------------------------

/// Build the Helm release name for a given slot.
pub fn release_name(base: &str, slot: Slot) -> String {
    match slot {
        Slot::Red => format!("{base}-red"),
        Slot::Black => format!("{base}-black"),
    }
}

/// Determine which slot should host the canary (new) release.
///
/// If there is an active slot, the canary goes into the opposite slot.
/// If there is no active slot (initial deployment), default to Red.
pub fn canary_slot(status: Option<&HelmReleaseStatus>) -> Slot {
    match status.and_then(|s| s.active_slot) {
        Some(active) => active.other(),
        None => Slot::Red,
    }
}

/// Record that a version has been installed into a slot.
pub fn set_slot_state(
    status: &mut HelmReleaseStatus,
    slot: Slot,
    version: String,
    release_name: String,
) {
    let state = SlotState {
        version,
        release_name,
    };

    let pair = status.slots.get_or_insert(SlotPair {
        red: None,
        black: None,
    });

    match slot {
        Slot::Red => pair.red = Some(state),
        Slot::Black => pair.black = Some(state),
    }
}

/// Clear the state of a slot (e.g. after uninstalling its release).
pub fn clear_slot_state(status: &mut HelmReleaseStatus, slot: Slot) {
    if let Some(pair) = &mut status.slots {
        match slot {
            Slot::Red => pair.red = None,
            Slot::Black => pair.black = None,
        }
    }
}

/// Get the state of a specific slot.
pub fn get_slot_state(status: &HelmReleaseStatus, slot: Slot) -> Option<&SlotState> {
    status.slots.as_ref().and_then(|pair| match slot {
        Slot::Red => pair.red.as_ref(),
        Slot::Black => pair.black.as_ref(),
    })
}

/// Swap the active slot to the given slot after a successful promotion.
///
/// Updates `active_slot` and `active_version` based on the slot's current
/// state. Returns the new active version, or `None` if the slot has no state.
pub fn promote_slot(status: &mut HelmReleaseStatus, new_active: Slot) -> Option<String> {
    let version = get_slot_state(status, new_active).map(|s| s.version.clone())?;

    status.active_slot = Some(new_active);
    status.active_version = Some(version.clone());

    Some(version)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_name_red() {
        assert_eq!(release_name("my-app", Slot::Red), "my-app-red");
    }

    #[test]
    fn release_name_black() {
        assert_eq!(release_name("my-app", Slot::Black), "my-app-black");
    }

    #[test]
    fn canary_slot_no_status() {
        assert_eq!(canary_slot(None), Slot::Red);
    }

    #[test]
    fn canary_slot_no_active() {
        let status = HelmReleaseStatus::default();
        assert_eq!(canary_slot(Some(&status)), Slot::Red);
    }

    #[test]
    fn canary_slot_active_red() {
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Red),
            ..Default::default()
        };
        assert_eq!(canary_slot(Some(&status)), Slot::Black);
    }

    #[test]
    fn canary_slot_active_black() {
        let status = HelmReleaseStatus {
            active_slot: Some(Slot::Black),
            ..Default::default()
        };
        assert_eq!(canary_slot(Some(&status)), Slot::Red);
    }

    #[test]
    fn set_and_get_slot_state() {
        let mut status = HelmReleaseStatus::default();
        set_slot_state(&mut status, Slot::Red, "1.0.0".into(), "my-app-red".into());

        let state = get_slot_state(&status, Slot::Red).expect("red slot should exist");
        assert_eq!(state.version, "1.0.0");
        assert_eq!(state.release_name, "my-app-red");

        assert!(get_slot_state(&status, Slot::Black).is_none());
    }

    #[test]
    fn set_both_slots() {
        let mut status = HelmReleaseStatus::default();
        set_slot_state(&mut status, Slot::Red, "1.0.0".into(), "my-app-red".into());
        set_slot_state(
            &mut status,
            Slot::Black,
            "2.0.0".into(),
            "my-app-black".into(),
        );

        assert_eq!(get_slot_state(&status, Slot::Red).unwrap().version, "1.0.0");
        assert_eq!(
            get_slot_state(&status, Slot::Black).unwrap().version,
            "2.0.0"
        );
    }

    #[test]
    fn clear_slot_state_removes() {
        let mut status = HelmReleaseStatus::default();
        set_slot_state(&mut status, Slot::Red, "1.0.0".into(), "my-app-red".into());
        clear_slot_state(&mut status, Slot::Red);
        assert!(get_slot_state(&status, Slot::Red).is_none());
    }

    #[test]
    fn clear_nonexistent_slot_is_noop() {
        let mut status = HelmReleaseStatus::default();
        clear_slot_state(&mut status, Slot::Red); // no panic
    }

    #[test]
    fn promote_slot_swaps_active() {
        let mut status = HelmReleaseStatus::default();
        status.active_slot = Some(Slot::Red);
        status.active_version = Some("1.0.0".into());

        set_slot_state(
            &mut status,
            Slot::Black,
            "2.0.0".into(),
            "my-app-black".into(),
        );

        let version = promote_slot(&mut status, Slot::Black);
        assert_eq!(version, Some("2.0.0".into()));
        assert_eq!(status.active_slot, Some(Slot::Black));
        assert_eq!(status.active_version, Some("2.0.0".into()));
    }

    #[test]
    fn promote_slot_initial_deployment() {
        let mut status = HelmReleaseStatus::default();
        set_slot_state(&mut status, Slot::Red, "1.0.0".into(), "my-app-red".into());

        let version = promote_slot(&mut status, Slot::Red);
        assert_eq!(version, Some("1.0.0".into()));
        assert_eq!(status.active_slot, Some(Slot::Red));
    }

    #[test]
    fn promote_slot_missing_state_returns_none() {
        let mut status = HelmReleaseStatus::default();
        let version = promote_slot(&mut status, Slot::Red);
        assert_eq!(version, None);
        assert!(status.active_slot.is_none());
    }

    #[test]
    fn overwrite_slot_state() {
        let mut status = HelmReleaseStatus::default();
        set_slot_state(&mut status, Slot::Red, "1.0.0".into(), "my-app-red".into());
        set_slot_state(&mut status, Slot::Red, "3.0.0".into(), "my-app-red".into());

        assert_eq!(get_slot_state(&status, Slot::Red).unwrap().version, "3.0.0");
    }
}

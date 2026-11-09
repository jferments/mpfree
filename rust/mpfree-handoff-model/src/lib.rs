//! **Simulation only.** This crate has no imports for file, process, OS,
//! systemd, ALSA, DRM, networking or persistent state. It cannot stop or
//! replace the stock MPC application. All evidence is mocked/owner-provided.
//! Prove state-machine safety here BEFORE developing privileged device code.
#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    StockRunning,
    Qualified,
    Armed,
    StockQuiescing,
    ResourcesReleased,
    MpFreeActive,
    StockRestoring,
    StockVerified,
    /// Real-world manual recovery would be needed; the model does not do it.
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidTransition,
    MissingRecoveryEvidence,
    InvalidDeadline,
    TimeWentBackwards,
    RestorationUnverified,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SafetyEvidence {
    pub board_firmware_matched: bool,
    pub independent_boot_restore_proven: bool,
    pub stock_project_saved: bool,
    pub owner_consented_to_interruption: bool,
    pub watchdog_separate_from_audio: bool,
    pub mock_restore_succeeded: bool,
}
impl SafetyEvidence {
    pub fn complete(&self) -> bool {
        self.board_firmware_matched
            && self.independent_boot_restore_proven
            && self.stock_project_saved
            && self.owner_consented_to_interruption
            && self.watchdog_separate_from_audio
            && self.mock_restore_succeeded
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StockRestoreChecks {
    pub stock_unit_active: bool,
    pub expected_factory_executable: bool,
    pub pcm_reacquired: bool,
    pub input_and_display_reacquired: bool,
    pub owner_confirmed_stock_ui: bool,
}
impl StockRestoreChecks {
    fn complete(self) -> bool {
        self.stock_unit_active
            && self.expected_factory_executable
            && self.pcm_reacquired
            && self.input_and_display_reacquired
            && self.owner_confirmed_stock_ui
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    OwnerCancel,
    MpFreeCrash,
    MissedHeartbeat,
    ExternalTransportFailure,
}

#[derive(Debug)]
pub struct HandoffSimulation {
    state: State,
    now_ms: u64,
    deadline_ms: Option<u64>,
    heartbeat_period_ms: u64,
    stop_reason: Option<StopReason>,
    restoration_attempts: u32,
}

impl Default for HandoffSimulation {
    fn default() -> Self {
        Self {
            state: State::StockRunning,
            now_ms: 0,
            deadline_ms: None,
            heartbeat_period_ms: 0,
            stop_reason: None,
            restoration_attempts: 0,
        }
    }
}

impl HandoffSimulation {
    pub fn state(&self) -> State { self.state }
    pub fn stop_reason(&self) -> Option<StopReason> { self.stop_reason }
    pub fn restoration_attempts(&self) -> u32 { self.restoration_attempts }

    /// Proof requirements are stricter than merely being able to SSH the MPC.
    pub fn qualify(&mut self, evidence: SafetyEvidence) -> Result<(), Error> {
        if self.state != State::StockRunning { return Err(Error::InvalidTransition); }
        if !evidence.complete() { return Err(Error::MissingRecoveryEvidence); }
        self.state = State::Qualified;
        Ok(())
    }

    pub fn arm(&mut self, period_ms: u64) -> Result<(), Error> {
        if self.state != State::Qualified { return Err(Error::InvalidTransition); }
        if !(10..=10_000).contains(&period_ms) { return Err(Error::InvalidDeadline); }
        self.heartbeat_period_ms = period_ms;
        self.state = State::Armed;
        Ok(())
    }

    /// No actual systemd interaction. The caller models its state.
    pub fn simulate_stock_quiesce(&mut self) -> Result<(), Error> {
        if self.state != State::Armed { return Err(Error::InvalidTransition); }
        self.state = State::StockQuiescing;
        Ok(())
    }

    pub fn simulate_resources_released(&mut self) -> Result<(), Error> {
        if self.state != State::StockQuiescing { return Err(Error::InvalidTransition); }
        self.state = State::ResourcesReleased;
        Ok(())
    }

    pub fn simulate_mpfree_start(&mut self, now_ms: u64) -> Result<(), Error> {
        if self.state != State::ResourcesReleased { return Err(Error::InvalidTransition); }
        let deadline = now_ms.checked_add(self.heartbeat_period_ms)
            .ok_or(Error::InvalidDeadline)?;
        if now_ms < self.now_ms { return Err(Error::TimeWentBackwards); }
        self.now_ms = now_ms;
        self.deadline_ms = Some(deadline);
        self.state = State::MpFreeActive;
        Ok(())
    }

    /// Strictly increasing clock; a late heartbeat CANNOT revive an expired session.
    pub fn heartbeat(&mut self, at_ms: u64) -> Result<(), Error> {
        if self.state != State::MpFreeActive { return Err(Error::InvalidTransition); }
        if at_ms < self.now_ms { return Err(Error::TimeWentBackwards); }
        let deadline = self.deadline_ms.ok_or(Error::InvalidTransition)?;
        self.now_ms = at_ms;
        if at_ms >= deadline {
            self.restore(StopReason::MissedHeartbeat);
            return Err(Error::InvalidDeadline);
        }
        let Some(next_deadline) = at_ms.checked_add(self.heartbeat_period_ms) else {
            // A clock arithmetic overflow must never leave a fake session active
            // without a new enforceable watchdog deadline.
            self.restore(StopReason::MissedHeartbeat);
            return Err(Error::InvalidDeadline);
        };
        self.deadline_ms = Some(next_deadline);
        Ok(())
    }

    pub fn advance_time(&mut self, at_ms: u64) -> Result<(), Error> {
        if at_ms < self.now_ms { return Err(Error::TimeWentBackwards); }
        self.now_ms = at_ms;
        if self.state == State::MpFreeActive
            && self.deadline_ms.is_some_and(|deadline| at_ms >= deadline)
        {
            self.restore(StopReason::MissedHeartbeat);
        }
        Ok(())
    }

    pub fn crash(&mut self) { self.restore(StopReason::MpFreeCrash); }
    pub fn transport_failure(&mut self) {
        self.restore(StopReason::ExternalTransportFailure);
    }
    pub fn cancel(&mut self) {
        match self.state {
            State::StockRunning | State::StockVerified | State::RecoveryRequired
                | State::StockRestoring => {}
            State::Qualified | State::Armed => {
                // No resource ownership transfer has happened; nothing to restore.
                self.state = State::StockRunning;
                self.deadline_ms = None;
            }
            _ => self.restore(StopReason::OwnerCancel),
        }
    }

    fn restore(&mut self, reason: StopReason) {
        match self.state {
            State::StockQuiescing | State::ResourcesReleased | State::MpFreeActive => {
                self.state = State::StockRestoring;
                self.stop_reason = Some(reason);
                self.deadline_ms = None;
                self.restoration_attempts += 1;
            }
            _ => {}
        }
    }

    /// Caller must substantiate all checks; false => recovery-required state.
    /// A mock success here is NEVER proof of a safe physical MPC handoff.
    pub fn simulate_stock_restored(
        &mut self, checks: StockRestoreChecks,
    ) -> Result<(), Error> {
        if self.state != State::StockRestoring { return Err(Error::InvalidTransition); }
        if !checks.complete() {
            self.state = State::RecoveryRequired;
            return Err(Error::RestorationUnverified);
        }
        self.state = State::StockVerified;
        Ok(())
    }

    pub fn simulate_return_to_stock(&mut self) -> Result<(), Error> {
        if self.state != State::StockVerified { return Err(Error::InvalidTransition); }
        self.state = State::StockRunning;
        self.stop_reason = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_gates() -> SafetyEvidence {
        SafetyEvidence {
            board_firmware_matched: true,
            independent_boot_restore_proven: true,
            stock_project_saved: true,
            owner_consented_to_interruption: true,
            watchdog_separate_from_audio: true,
            mock_restore_succeeded: true,
        }
    }
    fn complete_restore() -> StockRestoreChecks {
        StockRestoreChecks {
            stock_unit_active: true,
            expected_factory_executable: true,
            pcm_reacquired: true,
            input_and_display_reacquired: true,
            owner_confirmed_stock_ui: true,
        }
    }
    fn active() -> HandoffSimulation {
        let mut m = HandoffSimulation::default();
        m.qualify(all_gates()).unwrap();
        m.arm(100).unwrap();
        m.simulate_stock_quiesce().unwrap();
        m.simulate_resources_released().unwrap();
        m.simulate_mpfree_start(200).unwrap();
        m
    }

    #[test]
    fn default_is_stock_and_all_missing_gates_refuse_handoff() {
        let mut m = HandoffSimulation::default();
        assert_eq!(m.state(), State::StockRunning);
        assert_eq!(m.qualify(SafetyEvidence::default()), Err(Error::MissingRecoveryEvidence));
        assert_eq!(m.state(), State::StockRunning);
        assert_eq!(m.arm(100), Err(Error::InvalidTransition));
        assert_eq!(m.simulate_stock_quiesce(), Err(Error::InvalidTransition));
    }

    #[test]
    fn every_individual_gate_is_mandatory() {
        let mut fields = [all_gates(); 6];
        fields[0].board_firmware_matched = false;
        fields[1].independent_boot_restore_proven = false;
        fields[2].stock_project_saved = false;
        fields[3].owner_consented_to_interruption = false;
        fields[4].watchdog_separate_from_audio = false;
        fields[5].mock_restore_succeeded = false;
        for bad in fields {
            let mut m = HandoffSimulation::default();
            assert_eq!(m.qualify(bad), Err(Error::MissingRecoveryEvidence));
            assert_eq!(m.state(), State::StockRunning);
        }
    }

    #[test]
    fn cancel_before_quiesce_does_not_claim_restoration() {
        let mut m = HandoffSimulation::default();
        m.qualify(all_gates()).unwrap();
        m.arm(100).unwrap();
        m.cancel();
        assert_eq!(m.state(), State::StockRunning);
        assert_eq!(m.restoration_attempts(), 0);
    }

    #[test]
    fn crash_restores_stock_and_verifies() {
        let mut m = active();
        m.heartbeat(250).unwrap();
        m.crash();
        assert_eq!(m.state(), State::StockRestoring);
        assert_eq!(m.stop_reason(), Some(StopReason::MpFreeCrash));
        assert_eq!(m.restoration_attempts(), 1);
        m.simulate_stock_restored(complete_restore()).unwrap();
        m.simulate_return_to_stock().unwrap();
        assert_eq!(m.state(), State::StockRunning);
    }

    #[test]
    fn missed_heartbeat_triggers_restore_and_no_resurrection() {
        let mut m = active();
        m.advance_time(299).unwrap();
        assert_eq!(m.state(), State::MpFreeActive);
        m.advance_time(300).unwrap();
        assert_eq!(m.state(), State::StockRestoring);
        assert_eq!(m.heartbeat(301), Err(Error::InvalidTransition));
        assert_eq!(m.stop_reason(), Some(StopReason::MissedHeartbeat));
    }

    #[test]
    fn late_heartbeat_also_triggers_restore() {
        let mut m = active();
        assert_eq!(m.heartbeat(300), Err(Error::InvalidDeadline));
        assert_eq!(m.state(), State::StockRestoring);
    }

    #[test]
    fn failed_restore_remains_failed_and_cannot_rearm() {
        let mut m = active();
        m.transport_failure();
        let mut checks = complete_restore();
        checks.pcm_reacquired = false;
        assert_eq!(m.simulate_stock_restored(checks), Err(Error::RestorationUnverified));
        assert_eq!(m.state(), State::RecoveryRequired);
        assert_eq!(m.qualify(all_gates()), Err(Error::InvalidTransition));
        assert_eq!(m.simulate_return_to_stock(), Err(Error::InvalidTransition));
    }

    #[test]
    fn invalid_time_and_deadline_fail_closed() {
        let mut m = HandoffSimulation::default();
        m.qualify(all_gates()).unwrap();
        assert_eq!(m.arm(0), Err(Error::InvalidDeadline));
        assert_eq!(m.arm(10001), Err(Error::InvalidDeadline));
        m.arm(100).unwrap();
        m.simulate_stock_quiesce().unwrap();
        m.simulate_resources_released().unwrap();
        assert_eq!(m.simulate_mpfree_start(u64::MAX), Err(Error::InvalidDeadline));
        assert_eq!(m.state(), State::ResourcesReleased);
        m.simulate_mpfree_start(20).unwrap();
        assert_eq!(m.heartbeat(19), Err(Error::TimeWentBackwards));
        assert_eq!(m.state(), State::MpFreeActive);
    }

    #[test]
    fn heartbeat_deadline_overflow_cannot_leave_session_running() {
        let mut m = HandoffSimulation::default();
        m.qualify(all_gates()).unwrap();
        m.arm(100).unwrap();
        m.simulate_stock_quiesce().unwrap();
        m.simulate_resources_released().unwrap();
        m.simulate_mpfree_start(u64::MAX - 150).unwrap();
        assert_eq!(m.heartbeat(u64::MAX - 80), Err(Error::InvalidDeadline));
        assert_eq!(m.state(), State::StockRestoring);
        assert_eq!(m.stop_reason(), Some(StopReason::MissedHeartbeat));
    }

    #[test]
    fn cancel_after_release_attempts_restoration() {
        let mut m = HandoffSimulation::default();
        m.qualify(all_gates()).unwrap();
        m.arm(100).unwrap();
        m.simulate_stock_quiesce().unwrap();
        m.cancel();
        assert_eq!(m.state(), State::StockRestoring);
        assert_eq!(m.stop_reason(), Some(StopReason::OwnerCancel));
    }
}

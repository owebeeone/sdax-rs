//! Structural readiness waits for a value transfer, never for a synthetic body.

use super::state::{Machine, St};
use super::Effect;
use crate::plan::Kind;
use crate::report::{FaultKind, FaultLabel, Phase, TraceKind};
use crate::Error;

impl Machine {
    pub(super) fn request_publication(&mut self, n: usize) {
        self.slots[n].st = St::Publishing;
        self.slots[n].publication_pending = true;
        self.fx.push(Effect::PublishReady {
            node: self.t.nodes[n].key,
        });
    }

    pub(super) fn on_ready_published(
        &mut self,
        n: usize,
        result: Result<(), Error>,
    ) -> Result<(), &'static str> {
        if !matches!(self.t.nodes[n].kind, Kind::Join | Kind::Component) {
            return Err("publication acknowledgement for a non-structural node");
        }
        if !self.slots[n].publication_pending {
            return Err("publication acknowledgement without an outstanding request");
        }
        self.slots[n].publication_pending = false;
        // Cancellation or a sibling's failure may already have terminalized
        // this node. The host still owes the acknowledgement, but never Ready.
        if self.slots[n].st == St::Publishing {
            match result {
                Ok(()) => {
                    self.slots[n].st = St::Ready;
                    self.emit(n, TraceKind::Ready);
                }
                Err(error) => {
                    let phase = if self.t.nodes[n].kind == Kind::Component {
                        Phase::Prepare
                    } else {
                        Phase::Run
                    };
                    self.slots[n].st = St::Failed;
                    self.emit(n, TraceKind::Fail(phase, FaultLabel::Error));
                    let fault = self.fault(n, phase, FaultKind::Error(error));
                    self.faults.push(fault);
                    self.release_grants(n);
                    self.settle_or_skip_inner(n, Some(self.t.nodes[n].key));
                    // Apply the normal containing-scope policy without a body
                    // event or a component attempt fault in Slot::faults.
                    self.node_failed(n);
                }
            }
        }
        self.after_settle(n);
        self.sweep();
        Ok(())
    }
}

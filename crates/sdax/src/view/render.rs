//! The text rendering of a [`PlanView`](super::PlanView).
//!
//! The shape follows `Proposal.md` § A.5: a header, the nodes with their
//! resolved attributes, exactly the declared edges, the earliest-start layers
//! (marked as not being barriers), the release graph as a partial order, and
//! the `why` lines.

use super::{human, kind_label, policy_label, PlanView};
use std::fmt::Write;

pub(crate) fn render(v: &PlanView, out: &mut String) -> std::fmt::Result {
    writeln!(
        out,
        "plan {}   engine-semantics {}   mode: {}",
        v.name, v.semantics, v.mode
    )?;
    writeln!(
        out,
        "policy: {}   shutdown: {}",
        policy_label(v.policy),
        v.shutdown
    )?;

    writeln!(out, "nodes")?;
    let width = v
        .nodes
        .iter()
        .map(|n| n.path.to_string().len())
        .max()
        .unwrap_or(0);
    for n in &v.nodes {
        let mut line = format!(
            "  {:<width$}  {:<20}",
            n.path.to_string(),
            kind_label(n),
            width = width
        );
        if n.needs.is_empty() {
            line.push_str("needs —");
        } else {
            let names: Vec<String> = n.needs.iter().map(|p| p.to_string()).collect();
            let _ = write!(line, "needs {}", names.join(", "));
        }
        if !n.spawns.is_empty() {
            let names: Vec<String> = n.spawns.iter().map(|p| p.to_string()).collect();
            let _ = write!(line, "   spawns {}", names.join(", "));
        }
        for (k, val) in &n.attrs {
            let uninteresting = val == "—" || val == "no" || (*k == "cancel" && val == "drop");
            if !uninteresting {
                let _ = write!(line, "   {k}: {val}");
            }
        }
        writeln!(out, "{}", line.trim_end())?;
    }

    let edges: Vec<String> = v
        .edges
        .iter()
        .map(|e| format!("{}→{}", e.from, e.to))
        .collect();
    writeln!(out, "edges (declared, exactly): {}", edges.join(", "))?;

    writeln!(
        out,
        "layers (earliest start if every body took one tick; NOT barriers)"
    )?;
    for (i, layer) in v.layers().iter().enumerate() {
        let names: Vec<String> = layer.iter().map(|p| p.to_string()).collect();
        writeln!(out, "  {i}: {}", names.join(", "))?;
    }

    writeln!(
        out,
        "release graph (partial order; ‖ = unordered, may overlap)"
    )?;
    let order = v.release_order();
    let waves: Vec<String> = order
        .waves()
        .iter()
        .map(|w| {
            let names: Vec<String> = w.iter().map(|p| p.to_string()).collect();
            if names.len() > 1 {
                format!("({})", names.join(" ‖ "))
            } else {
                names.join("")
            }
        })
        .collect();
    writeln!(out, "  {}", waves.join(" → "))?;

    for n in &v.nodes {
        if let Some(why) = v.why(&n.path.to_string()) {
            if why.waits_on.is_empty() {
                continue;
            }
            let parts: Vec<String> = why
                .waits_on
                .iter()
                .map(|(p, r)| format!("{p} [{r}]"))
                .collect();
            writeln!(out, "why {} waits: {}", n.path, parts.join(", "))?;
        }
    }

    if !v.pools.is_empty() {
        writeln!(out, "pools")?;
        for p in &v.pools {
            let users: Vec<String> = p.users.iter().map(|u| u.to_string()).collect();
            writeln!(
                out,
                "  {} limit {}   users: {}",
                p.name,
                p.limit,
                users.join(", ")
            )?;
        }
    }
    let _ = human(std::time::Duration::from_secs(0));
    Ok(())
}

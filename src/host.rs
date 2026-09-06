//! Structured engine requests from scripts ([`HostEffect`]).
//! Does not go through stdout; `print("strata:…")` is the compatibility path.

use serde::{Deserialize, Serialize};

/// Structured request from a script to the host (engine).
/// `print("strata:…")` remains a compatibility path; these do not go through stdout.
///
/// Tagged as `type` (not `kind`) so `Spawn.kind` can mean entity kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HostEffect {
    Move {
        dx: f64,
        dy: f64,
    },
    Rot {
        degrees: f64,
    },
    Set {
        x: Option<f64>,
        y: Option<f64>,
        rot: Option<f64>,
    },
    Spawn {
        name: String,
        kind: String,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        color: String,
        script: Option<String>,
    },
    /// Clone a scene prefab. `x`/`y` are world coords when set; otherwise caller + prefab offset.
    SpawnPrefab {
        prefab: String,
        x: Option<f64>,
        y: Option<f64>,
    },
    Destroy {
        name: Option<String>,
    },
    PlaySound {
        name: Option<String>,
    },
    /// Fire this entity's named signal. `args` are JSON scalars for the host.
    Emit {
        signal: String,
        args: Vec<serde_json::Value>,
    },
    /// Call `method` on this entity after `delay` seconds. No async.
    After {
        delay: f64,
        method: String,
    },
    /// Screen-space play HUD. Immediate-mode: call every `on_update`.
    UiText {
        x: f64,
        y: f64,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabeledHostEffects {
    pub label: String,
    pub effects: Vec<HostEffect>,
}

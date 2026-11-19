#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

//! Fingerzinken und Schlitze für Laserschnitt-Boxen.
//!
//! Ein Aufruf von `make-joints` liest die gewählten Kanten, legt die gemeinsamen Parameter
//! `laser_finger`, `laser_kerf`, `laser_fit`, `laser_depth` und `laser_stock` an und baut je Kante
//! eine Skizze plus eine Extrusion. Das ist ein Undo-Schritt. Die Rechnung steht in `joints` und
//! `geom`; hier bleibt nur der Einstieg.

mod build;
mod geom;
mod joints;

#[cfg(target_arch = "wasm32")]
mod guest {
    wit_bindgen::generate!({ path: "wit", world: "command-plugin" });

    use super::build::{Fail, Runner};
    use craft::plugin::{
        commands, control, log,
        types::{FieldFailure, PluginError},
    };
    use serde_json::{json, Value};

    struct Host;

    impl Runner for Host {
        fn run(&mut self, command: &str, args: Value) -> Result<Value, String> {
            match commands::run(command, &args.to_string()) {
                Ok(reply) => serde_json::from_str(&reply)
                    .map_err(|e| format!("{command}: Antwort ist kein JSON ({e})")),
                Err(e) => Err(format!("{command}: {e:?}")),
            }
        }

        fn progress(&mut self, fraction: f32, message: &str) {
            control::progress(fraction, message);
        }

        fn cancelled(&mut self) -> bool {
            control::cancelled()
        }

        fn info(&mut self, message: &str) {
            log::log(log::Level::Info, message);
        }
    }

    struct Joints;

    impl exports::craft::plugin::entry::Guest for Joints {
        fn execute(entry: String, params: String) -> Result<String, PluginError> {
            if entry != "make-joints" {
                return Err(PluginError::InvalidInput(FieldFailure {
                    field: "entry".into(),
                    reason: format!("unbekannter Eintrag {entry}"),
                }));
            }
            let value: Value = serde_json::from_str(&params).unwrap_or(json!({}));
            match super::build::make(&mut Host, &value) {
                Ok(summary) => Ok(summary.to_string()),
                Err(Fail::Input { field, reason }) => {
                    Err(PluginError::InvalidInput(FieldFailure { field, reason }))
                }
                Err(Fail::Cancelled) => Err(PluginError::Cancelled),
                Err(Fail::Command(reason)) => Err(PluginError::Trap(reason)),
            }
        }
    }

    export!(Joints);
}

//! Data sent to the UI. These are deliberately separate from the `cf-core`
//! model: the domain crates stay free of UI concerns, and the IPC contract can
//! evolve without touching stored snapshots. TypeScript types are generated
//! from these structs into `app/src/bindings/` by `cargo test -p contextflow-app`.

mod cleanup;
mod settings;
mod workspace;

pub use cleanup::*;
pub use settings::*;
pub use workspace::*;

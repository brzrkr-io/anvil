//! Shell prompt rendering: git, toolchain, container/cluster, exit state.

pub mod build_segments;
pub mod ci;
pub mod context;
pub mod git;
pub mod icons;
pub mod kube;
pub mod render;
pub mod segments;

pub use ci::{CiState, CiStatus};
pub use kube::{EnvKind, KubeCtx};

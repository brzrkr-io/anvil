//! Shell prompt rendering: git, toolchain, container/cluster, exit state.

pub mod build_segments;
pub mod context;
pub mod git;
pub mod icons;
pub mod kube;
pub mod render;
pub mod segments;

pub use kube::{EnvKind, KubeCtx};

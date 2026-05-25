//! Kubernetes context detection: cluster name, namespace, environment kind.

/// Classifies a cluster by environment tier based on its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvKind {
    Prod,
    Staging,
    Dev,
}

impl EnvKind {
    pub fn from_cluster_name(s: &str) -> EnvKind {
        let lower = s.to_lowercase();
        if lower.contains("prod") || lower.contains("prd") {
            EnvKind::Prod
        } else if lower.contains("stg") || lower.contains("staging") {
            EnvKind::Staging
        } else {
            EnvKind::Dev
        }
    }
}

/// A parsed kubectl context: cluster name, namespace, and environment tier.
#[derive(Clone, Debug)]
pub struct KubeCtx {
    pub cluster: String,
    pub namespace: String,
    pub env_kind: EnvKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prod_cluster_name() {
        assert_eq!(
            EnvKind::from_cluster_name("gke-prod-us-east1"),
            EnvKind::Prod
        );
        assert_eq!(EnvKind::from_cluster_name("k8s-prd-cluster"), EnvKind::Prod);
    }

    #[test]
    fn staging_cluster_name() {
        assert_eq!(EnvKind::from_cluster_name("stg-cluster"), EnvKind::Staging);
        assert_eq!(EnvKind::from_cluster_name("staging-west"), EnvKind::Staging);
    }

    #[test]
    fn dev_cluster_name() {
        assert_eq!(EnvKind::from_cluster_name("kind-local"), EnvKind::Dev);
        assert_eq!(EnvKind::from_cluster_name("minikube"), EnvKind::Dev);
        assert_eq!(EnvKind::from_cluster_name("docker-desktop"), EnvKind::Dev);
    }
}

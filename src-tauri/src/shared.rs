use std::sync::Mutex;

// Selected AWS named profile (from Accounts), applied as AWS_PROFILE to kubectl
// so EKS auth uses the right credentials.
static AWS_PROFILE: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new();
pub(crate) fn aws_profile() -> &'static Mutex<String> {
    AWS_PROFILE.get_or_init(|| Mutex::new(String::new()))
}

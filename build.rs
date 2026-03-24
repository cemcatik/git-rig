fn main() {
    if std::env::var("CI").is_ok() || !std::path::Path::new(".git").exists() {
        return;
    }

    let hook = std::path::Path::new(".git/hooks/pre-commit");
    if !hook.exists() {
        std::fs::write(hook, include_str!("hooks/pre-commit"))
            .expect("failed to write pre-commit hook");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755))
                .expect("failed to set hook permissions");
        }
    }
}

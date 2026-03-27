use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

/// Returns the application config directory path, creating it if it doesn't exist.
///
/// On Unix-like systems (Linux, macOS): ~/.config/{app_name}
/// On Windows: %USERPROFILE%\{app_name}
///
/// # Arguments
/// * `app_name` - The name of your application (used as the directory name)
///
/// # Returns
/// * `Ok(PathBuf)` - The path to the config directory
/// * `Err(io::Error)` - If the directory cannot be created or home directory cannot be determined
///
/// # Example
/// ```ignore
/// let config_dir = get_or_create_config_dir("kimun")?;
/// println!("Config directory: {}", config_dir.display());
/// ```
pub fn get_or_create_config_dir(app_name: &str) -> io::Result<PathBuf> {
    let config_dir = get_config_dir_path(app_name)?;

    // Create the directory if it doesn't exist
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }

    Ok(config_dir)
}

/// Returns the application config directory path without creating it.
///
/// On Unix-like systems (Linux, macOS): ~/.config/{app_name}
/// On Windows: %USERPROFILE%\{app_name}
///
/// # Arguments
/// * `app_name` - The name of your application (used as the directory name)
///
/// # Returns
/// * `Ok(PathBuf)` - The path to the config directory
/// * `Err(io::Error)` - If the home directory cannot be determined
pub fn get_config_dir_path(app_name: &str) -> io::Result<PathBuf> {
    // Should I check for $XDG_CONFIG_HOME?
    let home_dir = get_home_dir()?;

    let config_dir = if cfg!(target_os = "windows") {
        // On Windows: %USERPROFILE%\{app_name}
        home_dir.join(app_name)
    } else {
        // On Unix-like systems: ~/.config/{app_name}
        home_dir.join(".config").join(app_name)
    };

    Ok(config_dir)
}

/// Gets the user's home directory.
fn get_home_dir() -> io::Result<PathBuf> {
    env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Could not determine home directory",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_get_home_dir() {
        let home = get_home_dir();
        assert!(home.is_ok(), "Should be able to get home directory");
        let home_path = home.unwrap();
        assert!(home_path.exists(), "Home directory should exist");
        assert!(home_path.is_dir(), "Home path should be a directory");
    }

    #[test]
    #[cfg(unix)]
    fn test_get_home_dir_unix() {
        let home = get_home_dir().unwrap();
        let home_env = env::var("HOME").unwrap();
        assert_eq!(home.to_str().unwrap(), home_env);
    }

    #[test]
    #[cfg(windows)]
    fn test_get_home_dir_windows() {
        let home = get_home_dir().unwrap();
        let home_env = env::var("USERPROFILE").unwrap();
        assert_eq!(home.to_str().unwrap(), home_env);
    }

    #[test]
    fn test_get_config_dir_path() {
        let config_path = get_config_dir_path("test_app").unwrap();

        #[cfg(target_os = "windows")]
        {
            assert!(config_path.ends_with("test_app"));
            assert!(!config_path.to_string_lossy().contains(".config"));
        }

        #[cfg(not(target_os = "windows"))]
        {
            assert!(config_path.ends_with(".config/test_app"));
            let path_str = config_path.to_string_lossy();
            assert!(path_str.contains(".config"));
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_get_config_dir_path_unix() {
        let config_path = get_config_dir_path("my_app").unwrap();
        let home = env::var("HOME").unwrap();
        let expected = PathBuf::from(home).join(".config").join("my_app");
        assert_eq!(config_path, expected);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_get_config_dir_path_macos() {
        let config_path = get_config_dir_path("macos_app").unwrap();
        assert!(config_path.to_string_lossy().contains(".config"));
        assert!(config_path.ends_with(".config/macos_app"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_get_config_dir_path_linux() {
        let config_path = get_config_dir_path("linux_app").unwrap();
        assert!(config_path.to_string_lossy().contains(".config"));
        assert!(config_path.ends_with(".config/linux_app"));
    }

    #[test]
    #[cfg(windows)]
    fn test_get_config_dir_path_windows() {
        let config_path = get_config_dir_path("windows_app").unwrap();
        let home = env::var("USERPROFILE").unwrap();
        let expected = PathBuf::from(home).join("windows_app");
        assert_eq!(config_path, expected);
        assert!(!config_path.to_string_lossy().contains(".config"));
    }

    #[test]
    fn test_get_or_create_config_dir() {
        let test_app_name = "test_app_temp_create_12345";
        let config_dir = get_or_create_config_dir(test_app_name).unwrap();

        assert!(
            config_dir.exists(),
            "Config directory should exist after creation"
        );
        assert!(config_dir.is_dir(), "Config path should be a directory");

        // Cleanup
        let _ = fs::remove_dir_all(config_dir);
    }

    #[test]
    fn test_get_or_create_config_dir_idempotent() {
        let test_app_name = "test_app_temp_idempotent_67890";

        // First call - creates directory
        let config_dir1 = get_or_create_config_dir(test_app_name).unwrap();
        assert!(config_dir1.exists());

        // Second call - should return same directory without error
        let config_dir2 = get_or_create_config_dir(test_app_name).unwrap();
        assert!(config_dir2.exists());
        assert_eq!(config_dir1, config_dir2);

        // Cleanup
        let _ = fs::remove_dir_all(config_dir1);
    }

    #[test]
    fn test_get_or_create_config_dir_nested() {
        let test_app_name = "test_app_nested/subdir/deep";
        let config_dir = get_or_create_config_dir(test_app_name).unwrap();

        assert!(config_dir.exists(), "Nested directory should be created");
        assert!(config_dir.is_dir(), "Path should be a directory");

        // Cleanup - remove the root test directory
        let root_test_dir = if cfg!(windows) {
            get_home_dir().unwrap().join("test_app_nested")
        } else {
            get_home_dir()
                .unwrap()
                .join(".config")
                .join("test_app_nested")
        };
        let _ = fs::remove_dir_all(root_test_dir);
    }

    #[test]
    #[cfg(unix)]
    fn test_directory_permissions_unix() {
        use std::os::unix::fs::PermissionsExt;

        let test_app_name = "test_app_permissions_unix";
        let config_dir = get_or_create_config_dir(test_app_name).unwrap();

        let metadata = fs::metadata(&config_dir).unwrap();
        let permissions = metadata.permissions();

        // Directory should be readable, writable, and executable by owner
        let mode = permissions.mode();
        assert!(
            mode & 0o700 != 0,
            "Owner should have read/write/execute permissions"
        );

        // Cleanup
        let _ = fs::remove_dir_all(config_dir);
    }

    #[test]
    fn test_app_name_with_special_characters() {
        let test_app_name = "test-app_123.config";
        let config_dir = get_or_create_config_dir(test_app_name).unwrap();

        assert!(config_dir.exists());
        assert!(config_dir.to_string_lossy().contains(test_app_name));

        // Cleanup
        let _ = fs::remove_dir_all(config_dir);
    }

    #[test]
    fn test_config_dir_path_without_creation() {
        let test_app_name = "test_app_no_create_98765";
        let config_path = get_config_dir_path(test_app_name).unwrap();

        // Path should be returned but not created
        // Note: This might fail if the directory already exists from previous runs
        // So we first ensure it doesn't exist
        if config_path.exists() {
            let _ = fs::remove_dir_all(&config_path);
        }

        let config_path = get_config_dir_path(test_app_name).unwrap();
        assert!(
            !config_path.exists(),
            "Directory should not be created by get_config_dir_path"
        );
    }
}

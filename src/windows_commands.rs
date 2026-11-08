//! Curated Windows settings and tools, adapted from Mycast.
pub struct Entry {
    pub id: &'static str,
    pub label: &'static str,
    pub executable: &'static str,
    pub argument: &'static str,
    pub keywords: &'static str,
}
pub const ENTRIES: &[Entry] = &[
    Entry {
        id: "installed-apps",
        label: "Installed apps",
        executable: "",
        argument: "ms-settings:appsfeatures",
        keywords: "add or remove programs|uninstall apps|apps and features",
    },
    Entry {
        id: "default-apps",
        label: "Default apps",
        executable: "",
        argument: "ms-settings:defaultapps",
        keywords: "file associations|browser",
    },
    Entry {
        id: "startup-apps",
        label: "Startup apps",
        executable: "",
        argument: "ms-settings:startupapps",
        keywords: "startup programs",
    },
    Entry {
        id: "display",
        label: "Display",
        executable: "",
        argument: "ms-settings:display",
        keywords: "screen|resolution|monitor",
    },
    Entry {
        id: "sound",
        label: "Sound",
        executable: "",
        argument: "ms-settings:sound",
        keywords: "volume|audio|microphone",
    },
    Entry {
        id: "bluetooth",
        label: "Bluetooth",
        executable: "",
        argument: "ms-settings:bluetooth",
        keywords: "devices|pair",
    },
    Entry {
        id: "wifi",
        label: "Wi-Fi",
        executable: "",
        argument: "ms-settings:network-wifi",
        keywords: "wireless|internet",
    },
    Entry {
        id: "ethernet",
        label: "Ethernet",
        executable: "",
        argument: "ms-settings:network-ethernet",
        keywords: "wired network",
    },
    Entry {
        id: "vpn",
        label: "VPN",
        executable: "",
        argument: "ms-settings:network-vpn",
        keywords: "virtual private network",
    },
    Entry {
        id: "proxy",
        label: "Network proxy",
        executable: "",
        argument: "ms-settings:network-proxy",
        keywords: "proxy server",
    },
    Entry {
        id: "windows-update",
        label: "Windows Update",
        executable: "",
        argument: "ms-settings:windowsupdate",
        keywords: "updates",
    },
    Entry {
        id: "storage",
        label: "Storage",
        executable: "",
        argument: "ms-settings:storagesense",
        keywords: "disk space|cleanup",
    },
    Entry {
        id: "power",
        label: "Power and battery",
        executable: "",
        argument: "ms-settings:powersleep",
        keywords: "sleep|battery",
    },
    Entry {
        id: "notifications",
        label: "Notifications",
        executable: "",
        argument: "ms-settings:notifications",
        keywords: "alerts",
    },
    Entry {
        id: "personalization",
        label: "Personalization",
        executable: "",
        argument: "ms-settings:personalization",
        keywords: "wallpaper|theme",
    },
    Entry {
        id: "taskbar",
        label: "Taskbar",
        executable: "",
        argument: "ms-settings:taskbar",
        keywords: "task bar",
    },
    Entry {
        id: "date-time",
        label: "Date and time",
        executable: "",
        argument: "ms-settings:dateandtime",
        keywords: "clock|timezone",
    },
    Entry {
        id: "language",
        label: "Language and region",
        executable: "",
        argument: "ms-settings:regionlanguage",
        keywords: "language|region",
    },
    Entry {
        id: "accessibility",
        label: "Accessibility",
        executable: "",
        argument: "ms-settings:easeofaccess",
        keywords: "ease of access",
    },
    Entry {
        id: "privacy",
        label: "Privacy and security",
        executable: "",
        argument: "ms-settings:privacy",
        keywords: "permissions",
    },
    Entry {
        id: "security",
        label: "Windows Security",
        executable: "",
        argument: "ms-settings:windowsdefender",
        keywords: "antivirus|defender",
    },
    Entry {
        id: "signin",
        label: "Sign-in options",
        executable: "",
        argument: "ms-settings:signinoptions",
        keywords: "password|pin|hello",
    },
    Entry {
        id: "about",
        label: "About your PC",
        executable: "",
        argument: "ms-settings:about",
        keywords: "specifications|system info",
    },
    Entry {
        id: "control-panel",
        label: "Control Panel",
        executable: "control.exe",
        argument: "",
        keywords: "control panel",
    },
    Entry {
        id: "programs-features",
        label: "Programs and Features",
        executable: "control.exe",
        argument: "appwiz.cpl",
        keywords: "classic uninstall",
    },
    Entry {
        id: "task-manager",
        label: "Task Manager",
        executable: "Taskmgr.exe",
        argument: "",
        keywords: "processes",
    },
    Entry {
        id: "device-manager",
        label: "Device Manager",
        executable: "mmc.exe",
        argument: "devmgmt.msc",
        keywords: "drivers|hardware",
    },
    Entry {
        id: "services",
        label: "Services",
        executable: "mmc.exe",
        argument: "services.msc",
        keywords: "windows services",
    },
    Entry {
        id: "disk-management",
        label: "Disk Management",
        executable: "mmc.exe",
        argument: "diskmgmt.msc",
        keywords: "partitions|disks",
    },
    Entry {
        id: "system-information",
        label: "System Information",
        executable: "msinfo32.exe",
        argument: "",
        keywords: "system specs",
    },
    Entry {
        id: "environment-variables",
        label: "Environment Variables",
        executable: "rundll32.exe",
        argument: "sysdm.cpl,EditEnvironmentVariables",
        keywords: "path|environment",
    },
];

/// IDs resolve only into the built-in table; user text never becomes arguments.
pub fn resolve(id: &str) -> Result<&'static Entry, String> {
    ENTRIES
        .iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| "Unknown Windows action".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_commands_are_unique_and_fixed() {
        let mut ids = std::collections::HashSet::new();
        assert_eq!(ENTRIES.len(), 31);
        for entry in ENTRIES {
            assert!(ids.insert(entry.id));
            assert!(!entry.label.is_empty());
            assert!(!entry.keywords.is_empty());
            if entry.executable.is_empty() {
                assert!(entry.argument.starts_with("ms-settings:"));
            } else {
                assert!(!entry.executable.contains(['/', '\\']));
                assert!(entry.executable.ends_with(".exe"));
            }
            assert_eq!(resolve(entry.id).unwrap().label, entry.label);
        }
        assert!(resolve("ms-settings:display").is_err());
        assert!(resolve("task-manager --arbitrary").is_err());
    }
}

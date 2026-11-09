#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Console-independent host capability facts and performance policy.
//!
//! This module does not probe the operating system or GPU. A host application
//! supplies detected capabilities, then asks the policy for a plan.
//! Emulation correctness must never depend on this plan.

/// Facts about resources and backends available to the host application.
///
/// Values are supplied by the host; this type intentionally performs no
/// platform-specific detection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostCapabilities {
    /// Number of logical CPU processors visible to the application.
    pub logical_cpu_count: usize,
    /// Physical memory available to this application, if known.
    pub available_memory_bytes: Option<u64>,
    /// Whether a usable hardware-accelerated video backend is available.
    pub hardware_video_backend: bool,
}

impl HostCapabilities {
    /// Creates a profile, treating a reported zero CPU count as one usable
    /// processor so policy decisions never produce a zero-thread plan.
    pub const fn new(
        logical_cpu_count: usize,
        available_memory_bytes: Option<u64>,
        hardware_video_backend: bool,
    ) -> Self {
        Self {
            logical_cpu_count: if logical_cpu_count == 0 {
                1
            } else {
                logical_cpu_count
            },
            available_memory_bytes,
            hardware_video_backend,
        }
    }
}

/// User-controlled constraints and preferences for host resource use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerformancePreferences {
    /// Optional upper bound for worker threads; None uses host capacity.
    pub max_worker_threads: Option<usize>,
    /// Prefer a hardware renderer when one is available.
    pub prefer_hardware_video: bool,
    /// Permit optional enhancements that may consume extra resources.
    ///
    /// This is a permission, not a promise that a particular core implements
    /// such enhancements. It must not change emulated hardware behavior.
    pub allow_optional_enhancements: bool,
    /// Optional cap for memory reserved by the emulator.
    pub memory_budget_bytes: Option<u64>,
}

impl Default for PerformancePreferences {
    fn default() -> Self {
        Self {
            max_worker_threads: None,
            prefer_hardware_video: true,
            allow_optional_enhancements: true,
            memory_budget_bytes: None,
        }
    }
}

/// Video backend selected by the host performance policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoBackendPreference {
    /// Use an available accelerated backend.
    Hardware,
    /// Use a software backend for compatibility or explicit preference.
    Software,
}

/// Resource allocation recommendation for a host runtime.
///
/// These are recommendations, not guarantees that all cores can parallelize
/// work. A core's deterministic execution and timing remain core-owned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerformancePlan {
    /// Recommended number of worker threads for independent host tasks.
    pub worker_threads: usize,
    /// Preferred video backend, with software as the portable fallback.
    pub video_backend: VideoBackendPreference,
    /// Memory reservation ceiling, if the host could report available memory.
    pub memory_budget_bytes: Option<u64>,
    /// Whether optional quality/performance features may be enabled.
    pub optional_enhancements_allowed: bool,
}

/// Derives a conservative resource plan from host facts and user preferences.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PerformancePolicy;

impl PerformancePolicy {
    /// Produces a plan without probing hardware or changing emulation behavior.
    pub const fn plan(
        capabilities: HostCapabilities,
        preferences: PerformancePreferences,
    ) -> PerformancePlan {
        let requested_threads = match preferences.max_worker_threads {
            Some(limit) if limit > 0 => limit,
            Some(_) => 1,
            None => capabilities.logical_cpu_count,
        };
        let worker_threads = if requested_threads < capabilities.logical_cpu_count {
            requested_threads
        } else {
            capabilities.logical_cpu_count
        };

        let video_backend =
            if preferences.prefer_hardware_video && capabilities.hardware_video_backend {
                VideoBackendPreference::Hardware
            } else {
                VideoBackendPreference::Software
            };

        let memory_budget_bytes = match (
            capabilities.available_memory_bytes,
            preferences.memory_budget_bytes,
        ) {
            (Some(available), Some(requested)) => {
                let conservative_available = available - available / 4;
                Some(if requested < conservative_available {
                    requested
                } else {
                    conservative_available
                })
            }
            (Some(available), None) => Some(available - available / 4),
            (None, requested) => requested,
        };

        PerformancePlan {
            worker_threads: if worker_threads == 0 {
                1
            } else {
                worker_threads
            },
            video_backend,
            memory_budget_bytes,
            optional_enhancements_allowed: preferences.allow_optional_enhancements,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_uses_host_parallelism_without_claiming_core_can_parallelize() {
        let plan = PerformancePolicy::plan(
            HostCapabilities::new(16, Some(8 * 1024 * 1024 * 1024), true),
            PerformancePreferences::default(),
        );
        assert_eq!(plan.worker_threads, 16);
        assert_eq!(plan.video_backend, VideoBackendPreference::Hardware);
        assert_eq!(plan.memory_budget_bytes, Some(6 * 1024 * 1024 * 1024));
        assert!(plan.optional_enhancements_allowed);
    }

    #[test]
    fn preferences_can_limit_threads_and_memory() {
        let plan = PerformancePolicy::plan(
            HostCapabilities::new(12, Some(4_000), true),
            PerformancePreferences {
                max_worker_threads: Some(3),
                prefer_hardware_video: true,
                allow_optional_enhancements: false,
                memory_budget_bytes: Some(10_000),
            },
        );
        assert_eq!(plan.worker_threads, 3);
        assert_eq!(plan.memory_budget_bytes, Some(3_000));
        assert!(!plan.optional_enhancements_allowed);
    }

    #[test]
    fn software_renderer_is_selected_when_acceleration_is_missing_or_unwanted() {
        let host = HostCapabilities::new(4, None, false);
        assert_eq!(
            PerformancePolicy::plan(host, PerformancePreferences::default()).video_backend,
            VideoBackendPreference::Software
        );
        assert_eq!(
            PerformancePolicy::plan(
                HostCapabilities::new(4, None, true),
                PerformancePreferences {
                    prefer_hardware_video: false,
                    ..PerformancePreferences::default()
                }
            )
            .video_backend,
            VideoBackendPreference::Software
        );
    }

    #[test]
    fn zero_reported_resources_never_produce_zero_worker_threads() {
        let plan = PerformancePolicy::plan(
            HostCapabilities::new(0, Some(0), false),
            PerformancePreferences {
                max_worker_threads: Some(0),
                ..PerformancePreferences::default()
            },
        );
        assert_eq!(plan.worker_threads, 1);
        assert_eq!(plan.memory_budget_bytes, Some(0));
    }

    #[test]
    fn unknown_memory_remains_unknown_unless_user_sets_a_budget() {
        let host = HostCapabilities::new(2, None, false);
        assert_eq!(
            PerformancePolicy::plan(host, PerformancePreferences::default()).memory_budget_bytes,
            None
        );
        assert_eq!(
            PerformancePolicy::plan(
                host,
                PerformancePreferences {
                    memory_budget_bytes: Some(1024),
                    ..PerformancePreferences::default()
                }
            )
            .memory_budget_bytes,
            Some(1024)
        );
    }
}

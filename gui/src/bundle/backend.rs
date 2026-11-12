//! The desktop app's update backend: the shared one (`SystemUpdates`), plus the
//! macOS bundle's own install (ADR-0056).
//!
//! The shared backend knows two ways to install: swap a raw binary, or hand the
//! user to the releases page (what it says for the `Bundle` channel). This
//! wrapper keeps everything else (checks, dismissing, the releases page, the
//! package-manager words) and answers `Install` for a bundle that can replace
//! itself, installing through a [`BundleInstaller`].

// `default_backend` builds none under `cfg(test)`.
#![cfg_attr(test, allow(dead_code))]

use std::sync::Arc;

use kimun_notes::logic::update::{UpdateBackend, UpdateOffer};
use kimun_notes::update::{InstallChannel, UpdateStatus};

/// Replaces the installed `Kimun.app` with the newest release's.
pub(crate) trait BundleInstaller: Send + Sync {
    /// Download, check and swap in the newest release's app: the version
    /// installed.
    fn install(&self) -> Result<String, String>;
}

/// `inner`'s backend, with the bundle's install added when `bundle` is there
/// (the running app sits in a bundle whose folder can be written to).
pub(crate) struct DesktopUpdates {
    inner: Arc<dyn UpdateBackend>,
    bundle: Option<Arc<dyn BundleInstaller>>,
}

impl DesktopUpdates {
    pub(crate) fn new(
        inner: Arc<dyn UpdateBackend>,
        bundle: Option<Arc<dyn BundleInstaller>>,
    ) -> Self {
        Self { inner, bundle }
    }

    /// The bundle installer, when this status is for the bundle channel.
    fn bundle_for(&self, status: &UpdateStatus) -> Option<&Arc<dyn BundleInstaller>> {
        self.bundle
            .as_ref()
            .filter(|_| status.channel == InstallChannel::Bundle)
    }
}

impl UpdateBackend for DesktopUpdates {
    fn check(&self, force: bool) -> Result<Option<UpdateStatus>, String> {
        self.inner.check(force)
    }

    fn install(&self, status: &UpdateStatus) -> Result<String, String> {
        match self.bundle_for(status) {
            Some(bundle) => bundle.install(),
            None => self.inner.install(status),
        }
    }

    fn dismiss(&self, version: &str) -> Result<(), String> {
        self.inner.dismiss(version)
    }

    fn releases_url(&self) -> String {
        self.inner.releases_url()
    }

    fn binary_published(&self) -> bool {
        self.inner.binary_published()
    }

    fn offer(&self, status: &UpdateStatus) -> UpdateOffer {
        match self.bundle_for(status) {
            Some(_) => UpdateOffer::Install,
            None => self.inner.offer(status),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use kimun_notes::logic::update::ScriptedUpdates;

    use super::*;

    const NEW: &str = "999.1.0";

    #[derive(Default)]
    struct FakeBundle {
        installs: AtomicUsize,
        fails: bool,
    }

    impl BundleInstaller for FakeBundle {
        fn install(&self) -> Result<String, String> {
            self.installs.fetch_add(1, Ordering::SeqCst);
            if self.fails {
                Err("signature refused".into())
            } else {
                Ok(NEW.into())
            }
        }
    }

    fn backend(
        channel: InstallChannel,
        bundle: Option<Arc<FakeBundle>>,
    ) -> (DesktopUpdates, Arc<ScriptedUpdates>) {
        let inner = Arc::new(ScriptedUpdates::with_release(NEW).channel(channel));
        let desktop =
            DesktopUpdates::new(inner.clone(), bundle.map(|b| b as Arc<dyn BundleInstaller>));
        (desktop, inner)
    }

    fn status(desktop: &DesktopUpdates) -> UpdateStatus {
        desktop.check(true).unwrap().expect("a status")
    }

    #[test]
    fn a_bundle_that_can_replace_itself_is_offered_the_install() {
        let (desktop, _) = backend(InstallChannel::Bundle, Some(Arc::default()));
        let s = status(&desktop);
        assert_eq!(desktop.offer(&s), UpdateOffer::Install);
    }

    #[test]
    fn a_bundle_that_cannot_stays_on_the_releases_page() {
        let (desktop, _) = backend(InstallChannel::Bundle, None);
        let s = status(&desktop);
        assert_eq!(desktop.offer(&s), UpdateOffer::Download);
    }

    #[test]
    fn other_channels_keep_the_shared_rules() {
        let bundle = Some(Arc::<FakeBundle>::default());
        let (brew, _) = backend(InstallChannel::Brew, bundle.clone());
        let s = status(&brew);
        assert!(matches!(brew.offer(&s), UpdateOffer::PackageManager { .. }));
        let (script, _) = backend(InstallChannel::Script, bundle);
        let s = status(&script);
        assert_eq!(script.offer(&s), UpdateOffer::Install);
    }

    #[test]
    fn a_bundle_install_goes_through_the_bundle_installer_only() {
        let bundle = Arc::<FakeBundle>::default();
        let (desktop, inner) = backend(InstallChannel::Bundle, Some(bundle.clone()));
        let s = status(&desktop);
        assert_eq!(desktop.install(&s), Ok(NEW.to_string()));
        assert_eq!(bundle.installs.load(Ordering::SeqCst), 1);
        assert_eq!(inner.installs(), 0, "the binary swap never ran");
    }

    #[test]
    fn a_bundle_install_failure_is_the_installers_words() {
        let bundle = Arc::new(FakeBundle {
            fails: true,
            ..FakeBundle::default()
        });
        let (desktop, _) = backend(InstallChannel::Bundle, Some(bundle));
        let s = status(&desktop);
        assert_eq!(desktop.install(&s), Err("signature refused".to_string()));
    }

    #[test]
    fn a_binary_install_goes_through_the_shared_backend() {
        let bundle = Arc::<FakeBundle>::default();
        let (desktop, inner) = backend(InstallChannel::Script, Some(bundle.clone()));
        let s = status(&desktop);
        assert!(desktop.install(&s).is_ok());
        assert_eq!(inner.installs(), 1);
        assert_eq!(bundle.installs.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn checks_and_the_rest_pass_through() {
        let (desktop, inner) = backend(InstallChannel::Bundle, Some(Arc::default()));
        desktop.check(true).unwrap();
        desktop.dismiss(NEW).unwrap();
        assert_eq!(inner.checks(), [true]);
        assert_eq!(inner.dismissed().as_deref(), Some(NEW));
        assert_eq!(desktop.releases_url(), inner.releases_url());
        assert_eq!(desktop.binary_published(), inner.binary_published());
    }
}

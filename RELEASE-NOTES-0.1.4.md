**Update check** (off by default)
- Settings → App → *Check GitHub for new versions*: about 20 seconds after start and then once a day, the app asks GitHub for the latest published release. *Check now* asks once, also while the switch is off.
- A newer version shows up as a bar in the window, an entry in the tray menu and a desktop notification. *What's new* shows the release notes, a download link for your package (`.deb` or AppImage, `amd64` or `arm64`) and how to install it; *Skip this version* hides it until the next one.
- Nothing is downloaded or installed by the app itself. The check sends one request to `api.github.com` and nothing else; a proxy from `HTTPS_PROXY`/`ALL_PROXY` is used.

**Packages**: `.deb` (Ubuntu 22.04, Debian 12 and newer) and AppImage (glibc 2.35+), for `amd64` and `arm64`.

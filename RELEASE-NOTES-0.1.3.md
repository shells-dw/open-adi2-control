**ADI-2-Pi connection**
- Access key: when an access key is set on the ADI-2-Pi, enter it under Settings → Connection → *Access key*. It is sent as header `X-ADI2PI-Key`. A missing or wrong key stops the connection and says so (no reconnect loop); after too many wrong keys the app waits a minute before trying again. The key is stored in `~/.config/open-adi2-control/config.json`, now readable by the user only.
- The unit's status message is read from `unitStatus` (ADI-2-Pi 1.0.0 and later); earlier bridges' `status` still works.
- The ADI-2-Pi search (*Find*) shows its results again.

**Window**
- Settings → Unit: the unit's status message updates live.

**Packages**: `.deb` (Ubuntu 22.04, Debian 12 and newer) and AppImage (glibc 2.35+), for `amd64` and `arm64`.

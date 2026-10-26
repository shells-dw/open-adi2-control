# Packaging

The release workflow builds `.deb`, `.rpm` and AppImage for amd64/x86_64 and arm64/aarch64.
This directory has what other package formats need.

| Path | |
|---|---|
| `linux/io.github.shells_dw.OpenAdi2Control.desktop` | desktop file (AUR, Flatpak) |
| `linux/io.github.shells_dw.OpenAdi2Control.metainfo.xml` | AppStream metadata |
| `aur/PKGBUILD`, `aur/.SRCINFO` | Arch Linux, built from the release tag |
| `flatpak/io.github.shells_dw.OpenAdi2Control.yml` | Flatpak of the released `.deb` |

## AUR

After the tag `vX.Y.Z` is pushed:

1. In `aur/PKGBUILD` set `pkgver`, `pkgrel=1`, then run `updpkgsums` (replaces `SKIP` with the
   tarball's sha256) and `makepkg --printsrcinfo > .SRCINFO`.
2. Test: `makepkg -si` in `aur/` on an Arch system.
3. Publish: clone `ssh://aur@aur.archlinux.org/open-adi2-control.git` (an AUR account with an
   SSH key is needed), copy `PKGBUILD` and `.SRCINFO` into it, commit, push.

The binary goes to `/usr/bin`, the headphone data to `/usr/lib/open-adi2-control/eq-data`
(Tauri looks for resources in `../lib/<name>` next to the binary).

## Flatpak

The manifest repackages the release `.deb` (built on Ubuntu 22.04). For each release set the
version in both URLs and the sha256 sums from `SHA256SUMS-amd64.txt` / `SHA256SUMS-arm64.txt`.

```sh
flatpak install flathub org.gnome.Platform//48 org.gnome.Sdk//48
flatpak-builder --user --install --force-clean build packaging/flatpak/io.github.shells_dw.OpenAdi2Control.yml
flatpak run io.github.shells_dw.OpenAdi2Control
```

The runtime has to contain WebKitGTK 4.1:
`flatpak run --command=sh org.gnome.Platform//48 -c 'ls /usr/lib/*/libwebkit2gtk-4.1.so.0'`.
If it doesn't, use a runtime version that does.

In the sandbox:

- USB MIDI needs `--device=all` (the ALSA sequencer device `/dev/snd/seq`); it is in the
  manifest.
- The volume keys work through the commands bound by the desktop
  (`flatpak run io.github.shells_dw.OpenAdi2Control volume-up` …). The GNOME key binding setup
  and the system-volume fallback (`wpctl`/`pactl` on the host) don't reach outside the sandbox.
- *Start at login* writes into the sandbox's autostart directory, which the desktop doesn't read.

# AutoLogin branding

The mascot is a friendly round robot "orb": a blue swirl sphere with a white cloud window and a navy visor showing cyan eyes. It holds a small key.

## Files

| File | Use |
|------|-----|
| `logo-source.png` | The original mascot artwork (raster, supplied by design). This is the preferred source for app icons when it is present. |
| `logo-mascot.svg` | Full mascot drawn as a vector: antennae, arms, key, feet and shadow, on a transparent background. Use it for the README, website, splash and about screens. |
| `logo-mark.svg` | App-icon mark: swirl orb, cloud window and visor on a light rounded-square tile. It stays readable at 32 px. |
| `logo-mark-1024.png` | `logo-mark.svg` rendered at 1024×1024. This was the input for the current `src-tauri/icons`. |
| `logo-mono.svg` | Single-colour glyph, black on transparent: orb ring, swirl arc and visor with the eyes knocked out. Use it for the macOS tray template image and the Android notification icon. |
| `tray-template.png`, `tray-template@2x.png` | `logo-mono.svg` at 22 px and 44 px, ready for a macOS template tray icon. |
| `android-adaptive-foreground.svg` | Android adaptive-icon foreground on a 108 dp canvas. The orb fits inside the 66 dp safe zone. Use background colour `#F1F4F8`, or `#FFFFFF`. |
| `logo-mascot-1024w.png` | `logo-mascot.svg` rendered 1024 px wide (transparent background). |

## Colours

| Token | Hex |
|-------|-----|
| Deep blue (orb body) | `#1E4FD8` |
| Royal blue | `#2F6BEF` |
| Periwinkle highlight | `#7FA2F0` |
| Visor navy | `#1B2A5C` |
| Glow cyan (eyes, key) | `#5FF2F2` |
| Background | `#F1F4F8` |

## Regenerating app icons

Run this from `app/`:

```sh
# Preferred: use the original artwork (square, ideally 1024x1024).
pnpm tauri icon branding/logo-source.png

# Fallback: use the vector mark.
rsvg-convert -w 1024 -h 1024 branding/logo-mark.svg -o branding/logo-mark-1024.png
pnpm tauri icon branding/logo-mark-1024.png
```

`tauri icon` writes the desktop icons to `src-tauri/icons/` (including `ios/`) and the Android mipmaps to `src-tauri/gen/android/.../res/mipmap-*`. It uses the full tile as the Android adaptive foreground, so the launcher mask crops it. After running it, replace the foregrounds with the safe-zone version:

```sh
cd src-tauri/gen/android/app/src/main/res
for d in mdpi:108 hdpi:162 xhdpi:216 xxhdpi:324 xxxhdpi:432; do
  rsvg-convert -w ${d#*:} -h ${d#*:} ../../../../../../../branding/android-adaptive-foreground.svg \
    -o mipmap-${d%%:*}/ic_launcher_foreground.png
done
```

## Tray icon

At present `src-tauri/src/app/tray.rs` uses `app.default_window_icon()`, which is the full-colour app icon. To get a monochrome macOS menu-bar icon, load `tray-template@2x.png` (copy it into `src-tauri/icons/` first) with `tauri::image::Image::from_bytes(include_bytes!(...))` and call `.icon_as_template(true)` on the `TrayIconBuilder`.

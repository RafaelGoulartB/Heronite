# Scoop

`heronite.json` is the manifest of Heronite's own Scoop bucket,
[`RafaelGoulartB/scoop-heronite`](https://github.com/RafaelGoulartB/scoop-heronite).
This copy is its source; the bucket keeps it up to date by itself.

## Installing and updating

```powershell
scoop bucket add heronite https://github.com/RafaelGoulartB/scoop-heronite
scoop install heronite/heronite
scoop update heronite   # after `scoop update`, which refreshes the buckets
```

Close Heronite, including from the tray, before updating: Scoop refuses
to replace a program that is still running.

## What the manifest does

- It unpacks `Heronite-Portable-x86_64.zip`, each version in its own folder
  `apps\heronite\<version>\`, with `apps\heronite\current` pointing at the
  installed one. It adds a Start menu shortcut and the `heronite` command.
- `post_install` takes `portable.txt` out, so the library and settings live
  in `%APPDATA%\heronite` and `%LOCALAPPDATA%\heronite` like an installed
  copy's, and survive updates without `persist`. It also writes `scoop` into
  `package-format`.
- With that marker, or Scoop's folders around the program, the app knows
  Scoop installed it (`heronite_platform::scoop`). It then only says that a
  new version is out (`scoop update heronite`) and never replaces its own
  files. Shortcuts, the autostart entry and `heronite://` links go through
  `current`, because `scoop cleanup` deletes the old version folders.
- Uninstalling removes what the app wrote to the registry (the `Run` value
  and `heronite://`), like the setup's uninstaller. The library stays.

## The bucket

1. Create `RafaelGoulartB/scoop-heronite` from
   [ScoopInstaller/BucketTemplate](https://github.com/ScoopInstaller/BucketTemplate)
   ("Use this template"). It brings the Excavator workflow and the checks.
2. Copy `heronite.json` into its `bucket/` folder and push.
3. In its Actions tab, run **Excavator** once. `checkver` finds the latest
   release, and `autoupdate` writes its URL and the SHA-256 from the
   release's `SHA256SUMS`. After that, Excavator runs every 4 hours, so new
   releases reach the bucket without anything to do here.

Never replace a file in a published release: its SHA-256 would no longer
match, and `scoop install` would refuse it. Fix a release with a new version
instead.

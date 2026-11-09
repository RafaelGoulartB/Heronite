# winget

Heronite is the package `Heronite.Heronite` of
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs):

```powershell
winget install Heronite.Heronite
winget upgrade Heronite.Heronite
```

It installs the release's `Heronite-Setup-x86_64.exe` (per user, silently),
the same setup as the download. The app's own updater keeps working: it runs
the new setup, which updates the version in Apps & features, so winget sees
it too.

## The first version, by hand

1. Fork `microsoft/winget-pkgs` on this GitHub account.
2. Write the manifests of a published release:
   `scripts/winget-manifest.sh 1.2.0` puts them in
   `dist/winget/manifests/h/Heronite/Heronite/1.2.0/`.
3. On Windows, check them and try them:
   ```powershell
   winget validate --manifest dist\winget\manifests\h\Heronite\Heronite\1.2.0
   winget settings --enable LocalManifestFiles   # once, as administrator
   winget install --manifest dist\winget\manifests\h\Heronite\Heronite\1.2.0
   ```
4. Copy the `manifests/h/Heronite/Heronite/1.2.0` folder into the fork, on
   a new branch, and open the pull request. Its bot installs the package,
   scans it with Microsoft Defender, and a moderator merges it.

The setup is not code-signed. If the bot reports `Validation-Defender-Error`,
submit the file as a false positive at
<https://www.microsoft.com/wdsi/filesubmission> and say so in the pull
request.

## Later versions

The `winget` job of `.github/workflows/release.yml` opens the pull request of
every new release
([winget-releaser](https://github.com/vedantmgoyal9/winget-releaser), from
the fork). It needs the repository secret `WINGET_TOKEN`: a classic token
with the `public_repo` scope. Without the secret, the job only says that it
is missing. It only works once the package is on winget-pkgs.

Never replace a file in a published release: winget checks the installer's
SHA-256. Fix a release with a new version instead.

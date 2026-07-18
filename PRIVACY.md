# Privacy Policy

Ayame Editor processes opened documents on the user's device. It does not
upload document contents, search terms, editing activity, or telemetry to the
project maintainers.

## Network activity

Ayame may contact GitHub for the following purposes:

- The desktop application checks the public Ayame Editor release metadata on
  startup when **Check for updates on startup** is enabled. This setting is
  enabled by default and can be disabled in Settings. Operators may also set
  `AYAME_NO_UPDATE_CHECK=1`.
- `ayame update`, and an update explicitly accepted in the desktop
  application, download a release artifact and its checksum from GitHub.

These requests disclose ordinary connection information, such as the user's IP
address and user agent, to GitHub. GitHub processes that information under the
[GitHub Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).

Ayame does not send opened document contents with these requests. Other network
activity only occurs when the user explicitly asks Ayame to access a network
location or starts a server that they configure.

## Local data

Settings, session state, recovery data, and temporary files are stored locally.
They remain under the user's control and are removed according to the
application's normal cleanup and uninstall behavior.

## Questions

For privacy questions, open an issue in the
[Ayame Editor repository](https://github.com/hjosugi/ayame-editor/issues)
without including private document contents or other sensitive information.

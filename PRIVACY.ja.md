# プライバシーポリシー

Ayame Editor は開いた document を user の device 上で処理します。Document の内容、
検索語、編集操作、telemetry を project maintainer へ upload しません。

## Network 動作

Ayame は次の目的で GitHub に接続する場合があります。

- Desktop application は、**起動時に update を確認**が有効な場合、起動時に Ayame
  Editor の公開 release metadata を確認します。この設定は既定で有効です。Settings
  で無効にするか、operator が `AYAME_NO_UPDATE_CHECK=1` を設定できます。
- `ayame update`、または desktop application で明示的に許可した update は、
  release artifact と checksum を GitHub から download します。

これらの request により、IP address や user agent など通常の接続情報が GitHub に
伝わります。GitHub は
[GitHub Privacy Statement](https://docs.github.com/ja/site-policy/privacy-policies/github-general-privacy-statement)
に基づいてその情報を処理します。

Ayame はこれらの request に開いた document の内容を含めません。それ以外の network
動作は、user が network location への access を明示的に指示した場合、または user
自身が設定した server を起動した場合に限られます。

## Local data

Settings、session state、recovery data、temporary file は local に保存されます。
これらは user の管理下にあり、application の通常の cleanup・uninstall 動作に従って
削除されます。

## 問い合わせ

Privacy に関する質問は、private document の内容など機密情報を含めずに
[Ayame Editor repository](https://github.com/hjosugi/ayame-editor/issues)で
issue を作成してください。

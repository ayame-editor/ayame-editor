cask "ayame" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.6.0"
  sha256 arm:   "14662605d2857675981ccb65b61da5936d0a1182d1e5dbfe266bd924ecb68e40",
         intel: "5752efddba3638df813a63650713af6e0a9f3bfa2856e6cb3441ac5820d7e199"

  url "https://github.com/hjosugi/ayame-editor/releases/download/v#{version}/ayame-v#{version}-macos-#{arch}.zip"
  name "Ayame Editor"
  desc "Desktop text editor for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"

  app "Ayame.app"

  zap trash: [
    "~/Library/Caches/ayame",
  ]
end

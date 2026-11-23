cask "ayame" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.10.3"
  sha256 arm:   "598754650042c655217c7992f902f3713bf8f10a106272ce05d0905c8564c584",
         intel: "2810c315f0f07bfd7a833954cafbf6445736d775a14defc6c74f68e7abb9b109"

  url "https://github.com/ayame-editor/ayame-editor/releases/download/v#{version}/ayame-v#{version}-macos-#{arch}.zip"
  name "Ayame Editor"
  desc "Desktop text editor for huge files"
  homepage "https://github.com/ayame-editor/ayame-editor"

  app "Ayame.app"

  zap trash: [
    "~/Library/Caches/ayame",
  ]
end

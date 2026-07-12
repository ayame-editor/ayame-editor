cask "ayame" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.7.0"
  sha256 arm:   "9cb9144ae1177289b1843240c226fdfcb4f01df874dd4705734f7ed0976a5c59",
         intel: "bc2d993888f1aaffe71765b38d91db4f029ee421e46179fe4a4c1b43681200b5"

  url "https://github.com/hjosugi/ayame-editor/releases/download/v#{version}/ayame-v#{version}-macos-#{arch}.zip"
  name "Ayame Editor"
  desc "Desktop text editor for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"

  app "Ayame.app"

  zap trash: [
    "~/Library/Caches/ayame",
  ]
end

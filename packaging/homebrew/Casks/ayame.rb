cask "ayame" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.5.17"
  sha256 arm:   "6c9b657d5bcd783693c609c60c7d25a1c0e181302e786e408265aa8d2264e37d",
         intel: "9a83596b98ae9263d0d69bb2a9ed60a95c0f6d8f362b03c0ea64a171f989652b"

  url "https://github.com/hjosugi/ayame-editor/releases/download/v#{version}/ayame-v#{version}-macos-#{arch}.zip"
  name "Ayame Editor"
  desc "Desktop text editor for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"

  app "Ayame.app"

  zap trash: [
    "~/Library/Caches/ayame",
  ]
end

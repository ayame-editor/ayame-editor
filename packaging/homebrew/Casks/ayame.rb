cask "ayame" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.7.5"
  sha256 arm:   "08e682ed0a89dd3345f143fdc1a3552ab197713f8a6640e3a54b87b69668416c",
         intel: "d505b3130ce2621198bbfb8eb030ffcf73873d5e6b7faee1a9e26e720e4dc18b"

  url "https://github.com/ayame-editor/ayame-editor/releases/download/v#{version}/ayame-v#{version}-macos-#{arch}.zip"
  name "Ayame Editor"
  desc "Desktop text editor for huge files"
  homepage "https://github.com/ayame-editor/ayame-editor"

  app "Ayame.app"

  zap trash: [
    "~/Library/Caches/ayame",
  ]
end

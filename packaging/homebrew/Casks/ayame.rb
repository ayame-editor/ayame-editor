cask "ayame" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.7.2"
  sha256 arm:   "1a0c98d96b4a940776246b079e60bf27fc743d0b8d541f2d7b9ce17c62fb6f2b",
         intel: "ace2f9613287458e633cfb9a19ca90bbe675d938423695203a1b8669e02954d0"

  url "https://github.com/hjosugi/ayame-editor/releases/download/v#{version}/ayame-v#{version}-macos-#{arch}.zip"
  name "Ayame Editor"
  desc "Desktop text editor for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"

  app "Ayame.app"

  zap trash: [
    "~/Library/Caches/ayame",
  ]
end

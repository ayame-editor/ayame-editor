cask "ayame" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.7.3"
  sha256 arm:   "4a39f2246f5fb17140f5dddb2a14813aa9fa5ec2ead9a90257f0ec64c31c4140",
         intel: "eda72da16abbcad4f26611d228338245f5e5c2d4269a5800ebc486388f7be8eb"

  url "https://github.com/hjosugi/ayame-editor/releases/download/v#{version}/ayame-v#{version}-macos-#{arch}.zip"
  name "Ayame Editor"
  desc "Desktop text editor for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"

  app "Ayame.app"

  zap trash: [
    "~/Library/Caches/ayame",
  ]
end

cask "ayame" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.5.18"
  sha256 arm:   "67a09c18cac99db296aa67d2ab0e1ee6a7004f88364618f312da393f3f547d27",
         intel: "c7163594df53e00e7dee6f2aa4b356dc82d0738aac74216381fd4458b3c85c55"

  url "https://github.com/hjosugi/ayame-editor/releases/download/v#{version}/ayame-v#{version}-macos-#{arch}.zip"
  name "Ayame Editor"
  desc "Desktop text editor for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"

  app "Ayame.app"

  zap trash: [
    "~/Library/Caches/ayame",
  ]
end

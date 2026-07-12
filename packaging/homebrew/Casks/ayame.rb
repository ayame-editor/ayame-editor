cask "ayame" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.6.1"
  sha256 arm:   "6d3181f32ad699e0ab12087e156727bf5ae9110e13d4688d83869b8f170d763a",
         intel: "efb2466725e210f97d47bcdd6cc616ba9d81115a4b3d53ddc9548a700972ed40"

  url "https://github.com/hjosugi/ayame-editor/releases/download/v#{version}/ayame-v#{version}-macos-#{arch}.zip"
  name "Ayame Editor"
  desc "Desktop text editor for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"

  app "Ayame.app"

  zap trash: [
    "~/Library/Caches/ayame",
  ]
end

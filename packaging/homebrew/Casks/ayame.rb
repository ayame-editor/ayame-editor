cask "ayame" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.7.4"
  sha256 arm:   "0fc1a58b2ca70dbe371ff688cce8917ed8391c02c05bcb8dc76c81f6b257cacf",
         intel: "954682b53f6f14fef000d6174c27d77a5ad90f0d203b4c395f55051e40c6bb5c"

  url "https://github.com/hjosugi/ayame-editor/releases/download/v#{version}/ayame-v#{version}-macos-#{arch}.zip"
  name "Ayame Editor"
  desc "Desktop text editor for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"

  app "Ayame.app"

  zap trash: [
    "~/Library/Caches/ayame",
  ]
end

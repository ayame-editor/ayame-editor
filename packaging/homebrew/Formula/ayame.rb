class Ayame < Formula
  desc "Desktop text editor and CLI tools for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"
  version "0.6.1"
  license "0BSD"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.6.1/ayame-v0.6.1-macos-aarch64.zip"
    sha256 "6d3181f32ad699e0ab12087e156727bf5ae9110e13d4688d83869b8f170d763a"
  elsif OS.mac?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.6.1/ayame-v0.6.1-macos-x86_64.zip"
    sha256 "efb2466725e210f97d47bcdd6cc616ba9d81115a4b3d53ddc9548a700972ed40"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.6.1/ayame-v0.6.1-linux-x86_64"
    sha256 "0acad457d6ded97c77dde38abfffcd1b61b07ffc1a8b475e5bf04be36731d180"
  else
    odie "Ayame prebuilt Homebrew formula currently supports macOS and Linux x86_64"
  end

  def install
    if OS.mac?
      bin.install "Ayame.app/Contents/MacOS/ayame"
    else
      bin.install cached_download => "ayame"
    end
  end

  test do
    assert_match "ayame #{version}", shell_output("#{bin}/ayame --version")
  end
end

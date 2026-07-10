class Ayame < Formula
  desc "Desktop text editor and CLI tools for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"
  version "0.5.18"
  license "0BSD"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.5.18/ayame-v0.5.18-macos-aarch64.zip"
    sha256 "67a09c18cac99db296aa67d2ab0e1ee6a7004f88364618f312da393f3f547d27"
  elsif OS.mac?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.5.18/ayame-v0.5.18-macos-x86_64.zip"
    sha256 "c7163594df53e00e7dee6f2aa4b356dc82d0738aac74216381fd4458b3c85c55"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.5.18/ayame-v0.5.18-linux-x86_64"
    sha256 "f26dd825427ae5b7389a7925184a8c2a55876c3420ba47aa2dfd71f1675ca7d4"
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

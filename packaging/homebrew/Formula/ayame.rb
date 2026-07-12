class Ayame < Formula
  desc "Desktop text editor and CLI tools for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"
  version "0.6.0"
  license "0BSD"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.6.0/ayame-v0.6.0-macos-aarch64.zip"
    sha256 "14662605d2857675981ccb65b61da5936d0a1182d1e5dbfe266bd924ecb68e40"
  elsif OS.mac?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.6.0/ayame-v0.6.0-macos-x86_64.zip"
    sha256 "5752efddba3638df813a63650713af6e0a9f3bfa2856e6cb3441ac5820d7e199"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.6.0/ayame-v0.6.0-linux-x86_64"
    sha256 "a7fca7b1eb00006fce6207d7363d3ffd227df33a5088677020b3af2ceb0a6a4d"
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

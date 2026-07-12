class Ayame < Formula
  desc "Desktop text editor and CLI tools for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"
  version "0.7.0"
  license "0BSD"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.0/ayame-v0.7.0-macos-aarch64.zip"
    sha256 "9cb9144ae1177289b1843240c226fdfcb4f01df874dd4705734f7ed0976a5c59"
  elsif OS.mac?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.0/ayame-v0.7.0-macos-x86_64.zip"
    sha256 "bc2d993888f1aaffe71765b38d91db4f029ee421e46179fe4a4c1b43681200b5"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.0/ayame-v0.7.0-linux-x86_64"
    sha256 "3a7ee399aef84899aa88bfe0fdd1babd7379810f4d8ab0dd9233a50013a25c9f"
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

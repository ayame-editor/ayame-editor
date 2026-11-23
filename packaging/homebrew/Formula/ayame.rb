class Ayame < Formula
  desc "Desktop text editor and CLI tools for huge files"
  homepage "https://github.com/ayame-editor/ayame-editor"
  version "0.10.3"
  license "0BSD"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/ayame-editor/ayame-editor/releases/download/v0.10.3/ayame-v0.10.3-macos-aarch64.zip"
    sha256 "598754650042c655217c7992f902f3713bf8f10a106272ce05d0905c8564c584"
  elsif OS.mac?
    url "https://github.com/ayame-editor/ayame-editor/releases/download/v0.10.3/ayame-v0.10.3-macos-x86_64.zip"
    sha256 "2810c315f0f07bfd7a833954cafbf6445736d775a14defc6c74f68e7abb9b109"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/ayame-editor/ayame-editor/releases/download/v0.10.3/ayame-v0.10.3-linux-x86_64"
    sha256 "7fc749f3b217c39cfc218b5acf7f53d8707275568dd731d1b78dc01eed7f797e"
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

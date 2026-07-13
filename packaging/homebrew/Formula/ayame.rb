class Ayame < Formula
  desc "Desktop text editor and CLI tools for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"
  version "0.7.3"
  license "0BSD"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.3/ayame-v0.7.3-macos-aarch64.zip"
    sha256 "4a39f2246f5fb17140f5dddb2a14813aa9fa5ec2ead9a90257f0ec64c31c4140"
  elsif OS.mac?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.3/ayame-v0.7.3-macos-x86_64.zip"
    sha256 "eda72da16abbcad4f26611d228338245f5e5c2d4269a5800ebc486388f7be8eb"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.3/ayame-v0.7.3-linux-x86_64"
    sha256 "5531ba955e2898bd2addec2d4dc52e6ccc81a5fede149785cd8b1002a227776a"
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

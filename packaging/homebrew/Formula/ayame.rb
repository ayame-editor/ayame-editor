class Ayame < Formula
  desc "Desktop text editor and CLI tools for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"
  version "0.7.1"
  license "0BSD"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.1/ayame-v0.7.1-macos-aarch64.zip"
    sha256 "85fdba1e60809270d38547a3a1e493aa1e6eb81b65c88a6b8ee51e48bb34cbdf"
  elsif OS.mac?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.1/ayame-v0.7.1-macos-x86_64.zip"
    sha256 "ec5f695f27c720245fbb488565a0c1b69c443643174d7c221907b37191c699b2"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.1/ayame-v0.7.1-linux-x86_64"
    sha256 "7ef8256ec54b4618f4309c5bb5da38d87ab2ab434c1cb0c8cd32a895379e2e1f"
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

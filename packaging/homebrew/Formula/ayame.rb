class Ayame < Formula
  desc "Desktop text editor and CLI tools for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"
  version "0.5.17"
  license "0BSD"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.5.17/ayame-v0.5.17-macos-aarch64.zip"
    sha256 "6c9b657d5bcd783693c609c60c7d25a1c0e181302e786e408265aa8d2264e37d"
  elsif OS.mac?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.5.17/ayame-v0.5.17-macos-x86_64.zip"
    sha256 "9a83596b98ae9263d0d69bb2a9ed60a95c0f6d8f362b03c0ea64a171f989652b"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.5.17/ayame-v0.5.17-linux-x86_64"
    sha256 "e8d185ccaa8252a7efbbe3bf9af6a9bd3248cb81d278ee56dcdaf575d1673414"
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

class Ayame < Formula
  desc "Desktop text editor and CLI tools for huge files"
  homepage "https://github.com/ayame-editor/ayame-editor"
  version "0.7.5"
  license "0BSD"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/ayame-editor/ayame-editor/releases/download/v0.7.5/ayame-v0.7.5-macos-aarch64.zip"
    sha256 "08e682ed0a89dd3345f143fdc1a3552ab197713f8a6640e3a54b87b69668416c"
  elsif OS.mac?
    url "https://github.com/ayame-editor/ayame-editor/releases/download/v0.7.5/ayame-v0.7.5-macos-x86_64.zip"
    sha256 "d505b3130ce2621198bbfb8eb030ffcf73873d5e6b7faee1a9e26e720e4dc18b"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/ayame-editor/ayame-editor/releases/download/v0.7.5/ayame-v0.7.5-linux-x86_64"
    sha256 "402f16a584f6117a6cabf4a3aa96bfecef17a8a8906837b747374ed2a6d4b119"
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

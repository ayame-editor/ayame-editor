class Ayame < Formula
  desc "Desktop text editor and CLI tools for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"
  version "0.7.2"
  license "0BSD"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.2/ayame-v0.7.2-macos-aarch64.zip"
    sha256 "1a0c98d96b4a940776246b079e60bf27fc743d0b8d541f2d7b9ce17c62fb6f2b"
  elsif OS.mac?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.2/ayame-v0.7.2-macos-x86_64.zip"
    sha256 "ace2f9613287458e633cfb9a19ca90bbe675d938423695203a1b8669e02954d0"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.2/ayame-v0.7.2-linux-x86_64"
    sha256 "3c9c83be12675057f12d3ac7e96b9981820e6917a0adf578a46de1a4606b9682"
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

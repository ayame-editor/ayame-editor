class Ayame < Formula
  desc "Desktop text editor and CLI tools for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"
  version "0.7.4"
  license "0BSD"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.4/ayame-v0.7.4-macos-aarch64.zip"
    sha256 "0fc1a58b2ca70dbe371ff688cce8917ed8391c02c05bcb8dc76c81f6b257cacf"
  elsif OS.mac?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.4/ayame-v0.7.4-macos-x86_64.zip"
    sha256 "954682b53f6f14fef000d6174c27d77a5ad90f0d203b4c395f55051e40c6bb5c"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/hjosugi/ayame-editor/releases/download/v0.7.4/ayame-v0.7.4-linux-x86_64"
    sha256 "c5cb119308211f74ba1c00bf6529a051e57622f3ea6800b70231e38856d3e687"
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

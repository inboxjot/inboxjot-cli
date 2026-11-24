class Inboxjot < Formula
  desc "Command-line interface for the Inboxjot email API"
  homepage "https://inboxjot.com/docs/cli"
  version "{{version}}"
  license "MIT"

  on_macos do
    on_arm do
      url "https://registry.npmjs.org/@inboxjot/cli-darwin-arm64/-/cli-darwin-arm64-{{version}}.tgz"
      sha256 "{{sha256:darwin-arm64}}"
    end
    on_intel do
      url "https://registry.npmjs.org/@inboxjot/cli-darwin-x64/-/cli-darwin-x64-{{version}}.tgz"
      sha256 "{{sha256:darwin-x64}}"
    end
  end

  on_linux do
    on_arm do
      url "https://registry.npmjs.org/@inboxjot/cli-linux-arm64/-/cli-linux-arm64-{{version}}.tgz"
      sha256 "{{sha256:linux-arm64}}"
    end
    on_intel do
      url "https://registry.npmjs.org/@inboxjot/cli-linux-x64/-/cli-linux-x64-{{version}}.tgz"
      sha256 "{{sha256:linux-x64}}"
    end
  end

  def install
    bin.install "bin/inboxjot"
    bin.install_symlink "inboxjot" => "ij"
    generate_completions_from_executable(bin/"inboxjot", "completion")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/inboxjot --version")
  end
end

class Jmux < Formula
  desc "Terminal multiplexer with project awareness and AI agent state indicators"
  homepage "https://github.com/joshholtz/jmux"
  url "https://github.com/joshholtz/jmux/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "MIT"
  head "https://github.com/joshholtz/jmux.git", branch: "main"

  bottle do
    sha256 cellar: :any_skip_relocation, arm64_sequoia: "placeholder"
    sha256 cellar: :any_skip_relocation, arm64_sonoma:  "placeholder"
    sha256 cellar: :any_skip_relocation, ventura:       "placeholder"
  end

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  def caveats
    <<~EOS
      Add jmux shell integration to your shell config:

        zsh:  echo 'eval "$(jmux init zsh)"' >> ~/.zshrc
        bash: echo 'eval "$(jmux init bash)"' >> ~/.bashrc

      Wire Claude Code hooks:
        jmux setup claude

      Wire Codex hooks:
        jmux setup codex
    EOS
  end

  test do
    assert_match "jmux", shell_output("#{bin}/jmux --help")
    system "#{bin}/jmux", "ls"
  end
end

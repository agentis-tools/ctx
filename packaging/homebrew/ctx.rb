# frozen_string_literal: true

# Homebrew formula for the prebuilt ctx CLI.
class Ctx < Formula
  desc "Fast CLI tool that generates AI-ready context from your codebase"
  homepage "https://docs.agentis.tools"
  version "0.3.3"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/agentis-tools/ctx/releases/download/v0.3.3/ctx-v0.3.3-aarch64-apple-darwin.tar.gz"
      sha256 "684eef3d5c192fdb0ed7dbb351b020bc185cbef190af0dde449351c93c9cb5d6"
    else
      url "https://github.com/agentis-tools/ctx/releases/download/v0.3.3/ctx-v0.3.3-x86_64-apple-darwin.tar.gz"
      sha256 "d4655da67d239f254fb040a0a4bfadca379de310f236a7709cba962e6a5cef35"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/agentis-tools/ctx/releases/download/v0.3.3/ctx-v0.3.3-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "21b9ce79def9fd07b123f30936fe65608679cce3532b6f789313b6bd815b6273"
    end
  end

  def install
    bin.install "ctx-v#{version}-#{target_triple}/ctx"
  end

  def target_triple
    if OS.mac?
      Hardware::CPU.arm? ? "aarch64-apple-darwin" : "x86_64-apple-darwin"
    else
      "x86_64-unknown-linux-gnu"
    end
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/ctx --version")
  end
end

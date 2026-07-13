---
id: privacy
title: Privacy Policy
---

# Privacy Policy

**Effective date:** July 13, 2026

This policy explains how the ctx project handles information when you use the
ctx command-line tool, its Claude Code and Codex plugins, optional MCP server,
and this documentation website.

## Summary

ctx is local-first software. By default, ctx processes source code and project
metadata on your machine. The ctx project does not operate a hosted code
analysis service, does not include product telemetry, and does not receive your
source code, generated context, local index, prompts, or query results through
normal local use.

## Information processed locally

Depending on the commands you run, ctx may read and process:

- source files, file paths, symbols, and dependency relationships;
- Git metadata and diffs;
- configuration and rule files in your project;
- prompts or search queries supplied to ctx; and
- locally generated indexes, embeddings, snapshots, gate logs, and reports.

This information remains in your environment unless you explicitly select a
network-backed feature or share an output yourself. Local data is stored in
locations such as the project's `.ctx/` directory and is controlled by you.
Uninstalling ctx does not automatically delete project data; you can remove it
using your normal filesystem tools.

The official Claude Code and Codex plugins invoke the locally installed `ctx`
binary. They do not add telemetry or send project data to the ctx maintainers.

## Optional network activity

ctx makes network requests only for features that require them or that you
explicitly configure:

- **Update checks and self-update:** ctx may contact GitHub Releases to check
  for a newer version. `ctx self-update` downloads release files only when you
  invoke it. GitHub may receive standard request information such as your IP
  address, user agent, requested version, and platform artifact.
- **OpenAI embeddings:** when you select the OpenAI embedding provider and
  supply an `OPENAI_API_KEY`, ctx sends the text being embedded, including
  relevant source-code or query text, to OpenAI's API. OpenAI handles that data
  under its own terms and privacy policy.
- **Ollama or another configured endpoint:** ctx sends embedding inputs to the
  endpoint you configure. A local Ollama endpoint keeps that traffic local;
  a remote endpoint is governed by its operator.
- **Local embedding model download:** the local embedding provider may download
  model files from its model host on first use. This downloads model data; it
  does not upload your repository for embedding.
- **MCP and host applications:** if you enable the optional MCP feature, ctx
  returns requested codebase information to the MCP client you configured.
  That client and any model provider it uses handle the returned information
  under their own privacy terms.

Your operating system, package manager, Git host, AI host application, and
network provider may independently process information as part of their
services. ctx does not control those third parties.

## Documentation website

The ctx documentation site does not configure first-party analytics,
advertising trackers, or marketing cookies. Its hosting and delivery providers
may process ordinary web-server information, such as IP addresses, request
times, requested pages, browser details, and security logs, under their own
policies. The ctx project does not use the documentation site to collect source
code, prompts, or local ctx indexes.

## Information you submit publicly

Information submitted to the public GitHub repository—including issues, pull
requests, discussions, and their attachments—is public and retained according
to GitHub's policies and the repository's history. Do not include credentials,
private source code, or other sensitive information in public submissions.

## Sharing and sale of information

The ctx project does not sell personal information and does not use ctx data
for advertising. Because the project does not receive data from normal local
use, it has no such data to share. Information is disclosed only when you
direct it to a third-party feature, when infrastructure providers process it as
described above, or when disclosure is legally required.

## Security and retention

You control the security and retention of locally stored ctx data. Review ignore
rules before indexing a repository, avoid indexing secrets, restrict filesystem
permissions as appropriate, and review generated context before sharing it.

No security measure is perfect. Please report suspected vulnerabilities
privately through the repository's GitHub security advisory interface rather
than a public issue.

## Children's privacy

ctx is a developer tool and is not directed to children under 13. The project
does not knowingly collect personal information from children.

## Changes to this policy

This policy may be updated as ctx changes. Material revisions will be published
in this repository and on the documentation site with a new effective date.

## Contact

For privacy questions, contact the maintainers at
[johan@saldestechnology.com](mailto:johan@saldestechnology.com) or open an issue
in the [ctx repository](https://github.com/agentis-tools/ctx/issues) if the
question does not contain sensitive information.

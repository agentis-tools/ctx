Name:           ctx
Version:        %{ctx_version}
Release:        1%{?dist}
Summary:        Fast CLI for generating AI-ready context from source code
License:        MIT OR Apache-2.0
URL:            https://github.com/agentis-tools/ctx
Source0:        ctx
Source1:        LICENSE-MIT
Source2:        LICENSE-APACHE

%description
ctx indexes, searches, and analyzes source trees for use by developers and
AI coding tools.

%install
install -Dpm 0755 %{SOURCE0} %{buildroot}%{_bindir}/ctx
install -Dpm 0644 %{SOURCE1} %{buildroot}%{_licensedir}/%{name}/LICENSE-MIT
install -Dpm 0644 %{SOURCE2} %{buildroot}%{_licensedir}/%{name}/LICENSE-APACHE

%files
%{_bindir}/ctx
%license %{_licensedir}/%{name}/LICENSE-MIT
%license %{_licensedir}/%{name}/LICENSE-APACHE

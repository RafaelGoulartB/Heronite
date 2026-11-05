# COPR's "make_srpm" method runs `make -f .copr/Makefile srpm outdir=... spec=...`
# in a Fedora chroot: it downloads Source0 (the GitHub tarball of the tag in
# the spec's Version) and makes the source RPM. See packaging/copr/README.md.
spec ?= packaging/copr/heronite.spec
outdir ?= .

srpm:
	dnf -y install rpm-build rpmdevtools
	spectool -g -C $(outdir) $(spec)
	rpmbuild -bs --define "_sourcedir $(outdir)" --define "_srcrpmdir $(outdir)" $(spec)

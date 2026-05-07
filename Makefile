# Makefile for the gct demo recording pipeline.
#
# Targets:
#   demos          rebuild gct and re-record all five GIFs
#   demos-hero     ditto, hero only
#   demos-f1       ditto, branch cleanup
#   demos-f2       ditto, search & filter
#   demos-f3       ditto, worktree post-create hooks
#   demos-f4       ditto, cross-repo Reviews
#
# Requires VHS (https://github.com/charmbracelet/vhs):
#   brew install vhs

TARGET := target/release/gct
TAPES_DIR := scripts/demo/tapes

VHS_ENV := GCT_REPO_ROOT=$(CURDIR) PATH=$(CURDIR)/target/release:$$PATH

.PHONY: demos demos-hero demos-f1 demos-f2 demos-f3 demos-f4

demos: demos-hero demos-f1 demos-f2 demos-f3 demos-f4

demos-hero: $(TARGET)
	$(VHS_ENV) vhs $(TAPES_DIR)/hero.tape

demos-f1: $(TARGET)
	$(VHS_ENV) vhs $(TAPES_DIR)/f1-cleanup.tape

demos-f2: $(TARGET)
	$(VHS_ENV) vhs $(TAPES_DIR)/f2-search.tape

demos-f3: $(TARGET)
	$(VHS_ENV) vhs $(TAPES_DIR)/f3-hooks.tape

demos-f4: $(TARGET)
	$(VHS_ENV) vhs $(TAPES_DIR)/cross-repo.tape

$(TARGET):
	cargo build --release

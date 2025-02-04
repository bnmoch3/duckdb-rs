.EXPORT_ALL_VARIABLES:

# In order to use buildtime_bindgen
# you need to build duckdb locally and export the envs
LD_LIBRARY_PATH=$(PWD)/lib:$LD_LIBRARY_PATH
DUCKDB_LIB_DIR=$(PWD)/lib
DUCKDB_INCLUDE_DIR=$(PWD)/lib
DUCKDB_STATIC=0

.PHONY: test test_buildtime_bindgen test_bundled lint

test: test_buildtime_bindgen

test_buildtime_bindgen:
	cargo test \
		--features=buildtime_bindgen,modern-full \
		-- --nocapture

lint:
	cargo clippy --all-targets --workspace \
		--features=buildtime_bindgen,modern-full \
		-- -D warnings -A clippy::redundant-closure

test_bundled:
	cargo test --features=bundled,modern-full -- --nocapture

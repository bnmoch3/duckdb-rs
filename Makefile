.EXPORT_ALL_VARIABLES:

# In order to use buildtime_bindgen
# you need to build duckdb locally and export the envs
LD_LIBRARY_PATH=$(PWD)/lib:$LD_LIBRARY_PATH
DUCKDB_LIB_DIR=$(PWD)/lib
DUCKDB_INCLUDE_DIR=$(PWD)/lib
DUCKDB_STATIC=0

all:
	cargo test --features buildtime_bindgen --features modern-full -- --nocapture
	cargo clippy --all-targets --workspace --features buildtime_bindgen --features modern-full -- -D warnings -A clippy::redundant-closure

test:
	cargo test --features bundled --features modern-full -- --nocapture

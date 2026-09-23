# DuckJeu v0.1 build & packaging.
#
# 打包路径与 extension-template-rs 约定一致：cargo 构建 cdylib，再用
# extension-ci-tools 的 append_extension_metadata.py 追加元数据。
# duckdb-rs 依赖 unstable C API，因此 ABI 类型为 C_STRUCT_UNSTABLE：
# 产物只保证与 TARGET_DUCKDB_VERSION 兼容。

EXTENSION_NAME   := duckjeu
EXTENSION_VERSION := 0.1.0
TARGET_DUCKDB_VERSION := v1.5.5

CARGO  ?= cargo
DUCKDB ?= duckdb

# 测试与示例使用项目内 venv（make venv 创建）；不存在时回退到系统 python3。
VENV        := .venv
VENV_PYTHON := $(VENV)/bin/python
PYTHON      ?= $(shell [ -x $(VENV_PYTHON) ] && echo $(VENV_PYTHON) || echo python3)

UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Darwin)
LIB_EXT := dylib
else
LIB_EXT := so
endif

LIB_DEBUG   := target/debug/lib$(EXTENSION_NAME).$(LIB_EXT)
LIB_RELEASE := target/release/lib$(EXTENSION_NAME).$(LIB_EXT)
OUT_DEBUG   := build/debug/$(EXTENSION_NAME).duckdb_extension
OUT_RELEASE := build/release/$(EXTENSION_NAME).duckdb_extension

METADATA := extension-ci-tools/scripts/append_extension_metadata.py

.PHONY: all release debug demo test contract-test live-test check fmt clippy venv clean

all: release

$(METADATA):
	@echo "extension-ci-tools submodule is missing; run: git submodule update --init"
	@exit 1

platform:
	@mkdir -p configure
	@$(DUCKDB) -noheader -list -c "PRAGMA platform;" > configure/platform.txt

release: platform
	$(CARGO) build --release
	@mkdir -p $(dir $(OUT_RELEASE))
	$(PYTHON) $(METADATA) -l $(LIB_RELEASE) -o $(OUT_RELEASE) -n $(EXTENSION_NAME) \
		-dv $(TARGET_DUCKDB_VERSION) -ev $(EXTENSION_VERSION) \
		-pf configure/platform.txt --abi-type C_STRUCT_UNSTABLE

debug: platform
	$(CARGO) build
	@mkdir -p $(dir $(OUT_DEBUG))
	$(PYTHON) $(METADATA) -l $(LIB_DEBUG) -o $(OUT_DEBUG) -n $(EXTENSION_NAME) \
		-dv $(TARGET_DUCKDB_VERSION) -ev $(EXTENSION_VERSION) \
		-pf configure/platform.txt --abi-type C_STRUCT_UNSTABLE

# 离线端到端演示：生成样例数据，注入绝对扩展路径后执行 demo.sql。
demo: release venv
	$(VENV_PYTHON) examples/make_sample_data.py
	@mkdir -p build
	@sed "s|@@EXTENSION@@|$(CURDIR)/$(OUT_RELEASE)|" examples/demo.sql > build/demo.sql
	$(DUCKDB) -unsigned -noheader -list -c ".read build/demo.sql"

# 真实服务验收：需要 DUCKJEU_API_KEY 或 TYPESAFE_API_KEY，以及 DUCKJEU_LIVE_TEST=1，否则记录为 blocked。
live-test: release venv
	$(VENV_PYTHON) test/sql/run_live_acceptance.py

venv: $(VENV_PYTHON)

$(VENV_PYTHON): requirements-dev.txt
	python3 -m venv $(VENV)
	$(VENV_PYTHON) -m pip install --quiet --upgrade pip
	$(VENV_PYTHON) -m pip install --quiet -r requirements-dev.txt

# 纯 Rust 契约测试（无需数据库或网络）
test:
	$(CARGO) test

# SQL 契约与错误路径测试（本地 HTTP stub，无真实网络调用）
contract-test: release venv
	$(VENV_PYTHON) test/sql/run_contract_tests.py

check: fmt clippy test

fmt:
	$(CARGO) fmt --check

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

clean:
	$(CARGO) clean
	rm -rf build configure

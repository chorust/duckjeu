#include "duckdb/main/capi/capi_internal.hpp"

extern "C" void *duckjeu_host_state_create();
extern "C" void *duckjeu_host_state_clone(void *state);
extern "C" void duckjeu_host_state_drop(void *state);
extern "C" void duckjeu_host_query_begin(void *state);
extern "C" void duckjeu_host_query_end(void *state, uint32_t error_type);
extern "C" uint32_t duckjeu_host_error_type(void *error_data);

namespace {

constexpr const char *STATE_KEY = "duckjeu.connection_runtime.v1";

class DuckJeuClientContextState final : public duckdb::ClientContextState {
public:
	explicit DuckJeuClientContextState(duckdb::ClientContext &context)
	    : runtime_state(duckjeu_host_state_create()) {
		// Scalar init runs after DuckDB has snapshotted states for the first QueryBegin.
		// Seed that initial observation window; subsequent queries use the native callback.
		duckjeu_host_query_begin(runtime_state);
	}

	~DuckJeuClientContextState() override {
		duckjeu_host_state_drop(runtime_state);
	}

	void QueryBegin(duckdb::ClientContext &) override {
		duckjeu_host_query_begin(runtime_state);
	}

	void QueryEnd(duckdb::ClientContext &, duckdb::optional_ptr<duckdb::ErrorData> error) override {
		uint32_t error_type = 0;
		if (error) {
			duckdb::ErrorDataWrapper error_copy{*error.get()};
			error_type = duckjeu_host_error_type(&error_copy);
		}
		duckjeu_host_query_end(runtime_state, error_type);
	}

	void *runtime_state;
};

} // namespace

extern "C" void *duckjeu_client_context_state_acquire(duckdb_client_context handle) {
	if (!handle) {
		return nullptr;
	}
	try {
		auto &context = reinterpret_cast<duckdb::CClientContextWrapper *>(handle)->context;
		auto *state_manager = context.registered_state.get();
		if (!state_manager) {
			return nullptr;
		}
		auto state = state_manager->GetOrCreate<DuckJeuClientContextState>(STATE_KEY, context);
		if (!state || !state.get()->runtime_state) {
			return nullptr;
		}
		return duckjeu_host_state_clone(state.get()->runtime_state);
	} catch (...) {
		return nullptr;
	}
}

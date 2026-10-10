import {
  op_bb_data_dir,
  op_bb_db_exec,
  op_bb_db_open,
  op_bb_db_query,
  op_bb_kv_delete,
  op_bb_kv_get,
  op_bb_kv_list,
  op_bb_kv_set,
  op_bb_log,
  op_bb_record,
  op_bb_sdk_call,
} from "ext:core/ops";

globalThis[Symbol.for("bb.host.ops")] = Object.freeze({
  op_bb_data_dir,
  op_bb_db_exec,
  op_bb_db_open,
  op_bb_db_query,
  op_bb_kv_delete,
  op_bb_kv_get,
  op_bb_kv_list,
  op_bb_kv_set,
  op_bb_log,
  op_bb_record,
  op_bb_sdk_call,
});

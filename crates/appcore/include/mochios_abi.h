#ifndef MOCHIOS_ABI_H
#define MOCHIOS_ABI_H

#include <stddef.h>
#include <stdint.h>

#define MOCHIOS_ABI_VERSION_MAJOR 1u
#define MOCHIOS_ABI_VERSION_MINOR 0u
#define MOCHIOS_ABI_VERSION_PATCH 0u
#define MOCHIOS_ABI_VERSION ((MOCHIOS_ABI_VERSION_MAJOR << 16) | (MOCHIOS_ABI_VERSION_MINOR << 8) | MOCHIOS_ABI_VERSION_PATCH)

#define MOCHIOS_STATUS_OK 0
#define MOCHIOS_STATUS_NULL_POINTER 1
#define MOCHIOS_STATUS_INVALID_UTF8 2
#define MOCHIOS_STATUS_INVALID_ARGUMENT 3
#define MOCHIOS_STATUS_BUFFER_TOO_SMALL 4
#define MOCHIOS_STATUS_UNSUPPORTED_PLATFORM 5
#define MOCHIOS_STATUS_SYSTEM_ERROR 6
#define MOCHIOS_STATUS_PANIC 255

#define MOCHIOS_ASSOCIATION_ROLE_VIEW ((uint16_t)1u << 0)
#define MOCHIOS_ASSOCIATION_ROLE_EDIT ((uint16_t)1u << 1)
#define MOCHIOS_ASSOCIATION_ROLE_ALL (MOCHIOS_ASSOCIATION_ROLE_VIEW | MOCHIOS_ASSOCIATION_ROLE_EDIT)

typedef struct MochiosStringView {
    const uint8_t *data;
    uint64_t length;
} MochiosStringView;

typedef struct MochiosMutableBuffer {
    uint8_t *data;
    uint64_t capacity;
} MochiosMutableBuffer;

typedef struct MochiosControlCenterItem MochiosControlCenterItem;
typedef struct MochiosAlert MochiosAlert;
typedef struct MochiosOpenPanel MochiosOpenPanel;
typedef struct MochiosSavePanel MochiosSavePanel;
typedef struct MochiosRecoveryStore MochiosRecoveryStore;
typedef struct MochiosRecoveryRecord MochiosRecoveryRecord;
typedef struct MochiosSessionStore MochiosSessionStore;
typedef struct MochiosApplicationSession MochiosApplicationSession;
typedef struct MochiosUndoManager MochiosUndoManager;

#ifdef __cplusplus
extern "C" {
#endif

uint32_t mochios_abi_version(void);
int64_t mochios_last_system_error(void);
int32_t mochios_last_status(void);
MochiosStringView mochios_status_name(int32_t status);
uint8_t mochios_last_result_has_value(void);
const uint8_t *mochios_last_result_string_data(void);
size_t mochios_last_result_string_length(void);
uint64_t mochios_last_result_u64(void);

int32_t mochios_clipboard_set_text(MochiosStringView text);
int32_t mochios_clipboard_set_text_utf8(const uint8_t *data, size_t length);
int32_t mochios_clipboard_read_text_utf8(void);
int32_t mochios_clipboard_copy_text(MochiosMutableBuffer output, uint64_t *required_length, uint8_t *has_text);

int32_t mochios_content_type_parse_utf8(const uint8_t *data, size_t length);
int32_t mochios_content_type_for_path_utf8(const uint8_t *data, size_t length);
int32_t mochios_content_type_for_extension_utf8(const uint8_t *data, size_t length);
uint8_t mochios_content_type_conforms_utf8(const uint8_t *value_data, size_t value_length, const uint8_t *parent_data, size_t parent_length);
int32_t mochios_content_type_preferred_extension_utf8(const uint8_t *data, size_t length);

int32_t mochios_association_set(MochiosStringView extension, MochiosStringView content_type, MochiosStringView bundle_id, uint16_t role_bits);
int32_t mochios_association_remove(MochiosStringView extension, MochiosStringView content_type, uint16_t role_bits);
int32_t mochios_association_resolve(MochiosStringView extension, MochiosStringView content_type, uint16_t role_bits, MochiosMutableBuffer output, uint64_t *required_length);
int32_t mochios_association_set_utf8(const uint8_t *extension_data, size_t extension_length, const uint8_t *content_type_data, size_t content_type_length, const uint8_t *bundle_id_data, size_t bundle_id_length, uint16_t role_bits);
int32_t mochios_association_remove_utf8(const uint8_t *extension_data, size_t extension_length, const uint8_t *content_type_data, size_t content_type_length, uint16_t role_bits);
int32_t mochios_association_resolve_utf8(const uint8_t *extension_data, size_t extension_length, const uint8_t *content_type_data, size_t content_type_length, uint16_t role_bits);

int32_t mochios_document_open(MochiosStringView path, MochiosStringView content_type, uint16_t role_bits, uint64_t *process_id);
int32_t mochios_document_open_with(MochiosStringView path, MochiosStringView content_type, MochiosStringView bundle_id, uint16_t role_bits, uint64_t *process_id);
int32_t mochios_document_open_utf8(const uint8_t *path_data, size_t path_length, const uint8_t *content_type_data, size_t content_type_length, uint16_t role_bits);
int32_t mochios_document_open_with_utf8(const uint8_t *path_data, size_t path_length, const uint8_t *content_type_data, size_t content_type_length, const uint8_t *bundle_id_data, size_t bundle_id_length, uint16_t role_bits);
int32_t mochios_notification_deliver_utf8(const uint8_t *bundle_id_data, size_t bundle_id_length, const uint8_t *title_data, size_t title_length, const uint8_t *body_data, size_t body_length);
MochiosControlCenterItem *mochios_control_center_item_create_utf8(const uint8_t *bundle_id_data, size_t bundle_id_length, const uint8_t *item_id_data, size_t item_id_length);
int32_t mochios_control_center_item_set_title_utf8(MochiosControlCenterItem *item, const uint8_t *data, size_t length);
int32_t mochios_control_center_item_add_row_utf8(MochiosControlCenterItem *item, const uint8_t *label_data, size_t label_length, const uint8_t *value_data, size_t value_length);
int32_t mochios_control_center_item_publish(MochiosControlCenterItem *item);
void mochios_control_center_item_destroy(MochiosControlCenterItem *item);
MochiosAlert *mochios_alert_create(void);
int32_t mochios_alert_present_error_utf8(MochiosAlert *alert, const uint8_t *title_data, size_t title_length, const uint8_t *message_data, size_t message_length);
int32_t mochios_alert_dismiss(MochiosAlert *alert);
uint8_t mochios_alert_is_visible(const MochiosAlert *alert);
void mochios_alert_destroy(MochiosAlert *alert);
MochiosOpenPanel *mochios_open_panel_create_utf8(const uint8_t *title_data, size_t title_length, const uint8_t *initial_data, size_t initial_length, const uint8_t *root_data, size_t root_length);
int32_t mochios_open_panel_show(MochiosOpenPanel *panel);
uint8_t mochios_open_panel_is_visible(const MochiosOpenPanel *panel);
int32_t mochios_open_panel_take_selection(MochiosOpenPanel *panel);
uint8_t mochios_open_panel_take_cancelled(MochiosOpenPanel *panel);
void mochios_open_panel_destroy(MochiosOpenPanel *panel);
MochiosSavePanel *mochios_save_panel_create_utf8(const uint8_t *title_data, size_t title_length, const uint8_t *suggested_data, size_t suggested_length, const uint8_t *initial_data, size_t initial_length, const uint8_t *root_data, size_t root_length, uint8_t confirms_replacement);
int32_t mochios_save_panel_show(MochiosSavePanel *panel);
uint8_t mochios_save_panel_is_visible(const MochiosSavePanel *panel);
int32_t mochios_save_panel_take_selection(MochiosSavePanel *panel);
uint8_t mochios_save_panel_take_cancelled(MochiosSavePanel *panel);
void mochios_save_panel_destroy(MochiosSavePanel *panel);
MochiosRecoveryStore *mochios_recovery_store_create_utf8(const uint8_t *data, size_t length);
int32_t mochios_recovery_store_save_utf8(MochiosRecoveryStore *store, const uint8_t *identifier_data, size_t identifier_length, const uint8_t *original_path_data, size_t original_path_length, const uint8_t *content_type_data, size_t content_type_length, uint64_t revision, const uint8_t *contents_data, size_t contents_length);
MochiosRecoveryRecord *mochios_recovery_store_load_utf8(MochiosRecoveryStore *store, const uint8_t *identifier_data, size_t identifier_length);
int32_t mochios_recovery_store_remove_utf8(MochiosRecoveryStore *store, const uint8_t *identifier_data, size_t identifier_length);
int32_t mochios_recovery_record_identifier(const MochiosRecoveryRecord *record);
int32_t mochios_recovery_record_original_path(const MochiosRecoveryRecord *record);
int32_t mochios_recovery_record_content_type(const MochiosRecoveryRecord *record);
uint64_t mochios_recovery_record_revision(const MochiosRecoveryRecord *record);
int32_t mochios_recovery_record_text_contents(const MochiosRecoveryRecord *record);
void mochios_recovery_record_destroy(MochiosRecoveryRecord *record);
void mochios_recovery_store_destroy(MochiosRecoveryStore *store);
MochiosSessionStore *mochios_session_store_create_utf8(const uint8_t *data, size_t length);
MochiosApplicationSession *mochios_application_session_create(void);
int32_t mochios_application_session_add_window_utf8(MochiosApplicationSession *session, const uint8_t *identifier_data, size_t identifier_length, const uint8_t *path_data, size_t path_length, const uint8_t *recovery_data, size_t recovery_length, uint8_t has_frame, float x, float y, float width, float height, uint8_t maximized, uint8_t fullscreen);
int32_t mochios_session_store_save(MochiosSessionStore *store, const MochiosApplicationSession *session);
MochiosApplicationSession *mochios_session_store_load(MochiosSessionStore *store);
size_t mochios_application_session_window_count(const MochiosApplicationSession *session);
int32_t mochios_session_store_clear(MochiosSessionStore *store);
void mochios_application_session_destroy(MochiosApplicationSession *session);
void mochios_session_store_destroy(MochiosSessionStore *store);
MochiosUndoManager *mochios_undo_manager_create(void);
int32_t mochios_undo_manager_begin_group_utf8(MochiosUndoManager *manager, const uint8_t *name_data, size_t name_length);
int32_t mochios_undo_manager_end_group(MochiosUndoManager *manager);
int32_t mochios_undo_manager_set_action_name_utf8(MochiosUndoManager *manager, const uint8_t *name_data, size_t name_length);
int32_t mochios_undo_manager_register_utf8(MochiosUndoManager *manager, const uint8_t *name_data, size_t name_length, uint64_t undo_action, uint64_t redo_action);
uint8_t mochios_undo_manager_can_undo(const MochiosUndoManager *manager);
uint8_t mochios_undo_manager_can_redo(const MochiosUndoManager *manager);
int32_t mochios_undo_manager_undo_action_name(const MochiosUndoManager *manager);
int32_t mochios_undo_manager_redo_action_name(const MochiosUndoManager *manager);
int32_t mochios_undo_manager_undo(MochiosUndoManager *manager);
int32_t mochios_undo_manager_redo(MochiosUndoManager *manager);
int32_t mochios_undo_manager_remove_all(MochiosUndoManager *manager);
int32_t mochios_undo_manager_set_levels(MochiosUndoManager *manager, size_t levels);
size_t mochios_undo_manager_grouping_level(const MochiosUndoManager *manager);
int32_t mochios_undo_manager_take_action(MochiosUndoManager *manager);
void mochios_undo_manager_destroy(MochiosUndoManager *manager);
int32_t mochios_application_request_exit(void);
int32_t mochios_application_request_close_key_window(void);
int32_t mochios_application_request_quit(void);

#ifdef __cplusplus
}
#endif

#endif

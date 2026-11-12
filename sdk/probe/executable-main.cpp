#include <mochi/executable.h>
#include <type_traits>
static_assert(std::is_same_v<decltype(&mochi_executable_open),uint32_t(*)(const char*,const char*)>);
static_assert(std::is_same_v<decltype(&mochi_executable_execve),int(*)(uint32_t,const char*const*,const char*const*)>);
static_assert(std::is_same_v<decltype(&mochi_executable_spawn),int(*)(mochi_spawned_process*,uint32_t,const posix_spawn_file_actions_t*,const posix_spawnattr_t*,const char*const*,const char*const*)>);
static_assert(std::is_same_v<decltype(&mochi_executable_close),int(*)(uint32_t)>);
extern "C" int sdk_executable_probe(void);
int main(){return sdk_executable_probe();}

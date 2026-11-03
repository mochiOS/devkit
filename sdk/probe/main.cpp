extern "C" int mochi_sdk_probe();
int main() { return mochi_sdk_probe() == 42 ? 0 : 1; }

#include <stdio.h>
extern int sdk_memory_probe(void);
int main(void)
{
    int result = sdk_memory_probe();
    printf("SDK current process memory probe result=%d\n", result);
    return result;
}

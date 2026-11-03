#define _GNU_SOURCE 1
#include <fnmatch.h>
#include <stdio.h>
#include <locale.h>
int mochi_fnmatch(const char *, const char *, int);
int main(void) {
    const char *patterns[] = {"*", "*.c", "?", "a?c", "[a-z]*", "[!a-z]*", "a\\*b", "[abc]", "[", "a*b*c", "a/*", "*.*"};
    const char *names[] = {"", "source.c", "A", "abc", "Z9", ".hidden", "a*b", "[", "axbyc", "a/b", "abc/def", "a.c"};
    int flags[] = {0, FNM_PATHNAME, FNM_NOESCAPE, FNM_PERIOD, FNM_CASEFOLD};
    setlocale(LC_ALL, "C");
    for (unsigned i=0; i<sizeof(patterns)/sizeof(*patterns); ++i)
      for (unsigned j=0; j<sizeof(names)/sizeof(*names); ++j)
        for (unsigned f=0; f<sizeof(flags)/sizeof(*flags); ++f) {
          int actual=mochi_fnmatch(patterns[i], names[j], flags[f]);
          int expected=fnmatch(patterns[i], names[j], flags[f]);
          if ((actual==0)!=(expected==0)) {
            fprintf(stderr, "Mismatch: %s / %s flags=%d\n", patterns[i],names[j],flags[f]); return 1;
          }
        }
    puts("720 filename matching cases agree with independent host libc (C locale).");
    return 0;
}

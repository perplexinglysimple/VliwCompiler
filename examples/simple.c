typedef unsigned long long u64;

int main(void) {
    volatile u64 *out = (volatile u64 *)0x100;
    *out = 42;
    return 0;
}

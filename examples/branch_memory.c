typedef unsigned long long u64;

int main(void) {
    volatile u64 *out = (volatile u64 *)0x140;

    u64 low = 0;
    u64 high = 0;
    u64 mixed = 7;

    for (u64 i = 0; i < 10; i = i + 1) {
        u64 term = (i + 2) * (i + 5);

        if (i < 5) {
            low = low + term;
            mixed = mixed ^ (term << 1);
            out[3] = low;
        } else {
            high = high + term;
            mixed = mixed + (term ^ i);
            out[4] = high;
        }

        if ((i & 1) == 0) {
            out[5] = mixed;
        } else {
            out[6] = low + high;
        }
    }

    out[0] = low;
    out[1] = high;
    out[2] = mixed;

    return 0;
}

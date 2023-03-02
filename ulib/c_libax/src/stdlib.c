#include <stdint.h>
#include <stdlib.h>

#include <libax.h>

void srand(unsigned s)
{
    ax_srand(s);
}

int rand(void)
{
    return ax_rand_u32();
}

_Noreturn void abort(void)
{
    ax_panic();
    __builtin_unreachable();
}

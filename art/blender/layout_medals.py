"""Author the stage's deterministic staggered mound, keeping all 252 medals."""
import random
import re
from pathlib import Path

root=Path(__file__).resolve().parents[2]
rng=random.Random(27)
rows=['    initial_medals: [']
for layer in range(3):
    for row in range(7):
        for col in range(12):
            x=-3.24+col*0.56+(row%2)*0.22+rng.uniform(-0.13,0.13)
            z=-3.8+row*0.51+layer*0.09+rng.uniform(-0.17,0.17)
            y=0.055+layer*0.09+rng.uniform(0,0.025)
            rows.append(f'        (origin: ({x:.3f}, {y:.3f}, {z:.3f}), columns: 1, rows: 1, layers: 1, spacing: (0.56, 0.09, 0.51)),')
rows.append('    ],')
path=root/'game/assets/data/stages/stage_01.stage.ron'
text=path.read_text(encoding='utf-8-sig')
path.write_text(re.sub(r'    initial_medals: \[.*?    \],','\n'.join(rows),text,flags=re.S),encoding='utf-8')
print('Authored 252 irregularly staggered medal positions.')

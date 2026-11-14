const {chromium}=require(process.env.PLAYWRIGHT_PATH || 'playwright');
const fs=require('fs'),path=require('path'),{pathToFileURL}=require('url');
(async()=>{
const browser=await chromium.launch({executablePath:'C:/Program Files/Google/Chrome/Application/chrome.exe',headless:true});
try {
const page=await browser.newPage({viewport:{width:1440,height:1000}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
const check=(v,m)=>{if(!v)throw Error(m)};
await page.goto(pathToFileURL(path.join(__dirname,'gallery.html')).href);
check(await page.locator('.card').count()===12,'12 cards');
await page.locator('[data-filter="opus"]').click();check(await page.locator('.card').count()===3,'model filter');
await page.locator('.search').fill('Trail');check(await page.locator('.card').count()===1,'search');
await page.locator('[data-save="opus-2"]').click();await page.reload();check(await page.locator('[data-save="opus-2"]').getAttribute('aria-pressed')==='true','favorite persisted');
await page.locator('.savefilter').click();check(await page.locator('.card').count()===1,'favorite filter');
await page.locator('.savefilter').click();
for(const theme of ['light','dark']){
await page.locator('#theme').selectOption(theme);
for(const img of await page.locator('.thumb img').all())await img.scrollIntoViewIfNeeded();
await page.waitForFunction(()=>[...document.querySelectorAll('.thumb img')].every(i=>i.complete&&i.naturalWidth>0));
check(await page.locator('.thumb img').count()===12,'all images '+theme);
}
await page.locator('#theme').selectOption('light');
await page.locator('[data-compare="opus-1"]').check();await page.locator('[data-compare="astra-1"]').check();
await page.locator('[data-compare="argon-1"]').click();
check(!await page.locator('[data-compare="argon-1"]').isChecked(),'third compare refused');
await page.locator('#compare').click();check(await page.locator('.pair img').count()===2,'comparison');
await page.locator('.close').click();await page.locator('#clear').click();
await page.locator('[data-open="opus-1"]').click();check(await page.locator('#viewer').isVisible(),'modal');
await page.locator('#next').click();check((await page.locator('#viewer-title').textContent()).includes('Trail'),'next');
await page.locator('[data-mode="live"]').click();
await page.frameLocator('iframe').locator('body').waitFor();
check((await page.locator('iframe').getAttribute('src')).includes('concept-2.html'),'live iframe');
await page.locator('.close').click();await page.waitForFunction(()=>!document.querySelector('iframe'));check(await page.locator('iframe').count()===0,'iframe cleaned');
await page.evaluate(()=>scrollTo(0,0));
await page.screenshot({path:path.join(__dirname,'gallery-desktop.png'),fullPage:true});
for(const width of [1100,390]){
await page.setViewportSize({width,height:900});check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'overflow '+width);
await page.screenshot({path:path.join(__dirname,'gallery-'+width+'.png'),fullPage:true});
}
check(errors.length===0,'JS errors '+errors.join(','));
console.log(JSON.stringify({status:'passed',checks:['12 cards','model/search filters','favorite persistence','24 theme images','comparison limit','two-up comparison','modal navigation','live local iframe','iframe cleanup','1100/390 responsive'],errors}));
}finally{await browser.close()}
})().catch(e=>{console.error(e);process.exit(1)});

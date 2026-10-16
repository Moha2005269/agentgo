// Marks every page it runs on.
const mark = document.createElement('p');
mark.id = 'ext-mark';
mark.textContent = 'extension ran on ' + location.pathname;
document.body.appendChild(mark);
